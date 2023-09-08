// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::gas_charger::GasCharger;
use move_binary_format::CompiledModule;
use move_bytecode_utils::module_cache::GetModule;
use move_core_types::account_address::AccountAddress;
use move_core_types::language_storage::{ModuleId, StructTag};
use move_core_types::resolver::{ModuleResolver, ResourceResolver};
use parking_lot::RwLock;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use sui_protocol_config::ProtocolConfig;
use sui_types::base_types::VersionDigest;
use sui_types::committee::EpochId;
use sui_types::effects::{TransactionEffects, TransactionEvents};
use sui_types::execution::{ExecutionResults, ExecutionResultsV2, LoadedChildObjectMetadata};
use sui_types::execution_status::ExecutionStatus;
use sui_types::inner_temporary_store::InnerTemporaryStore;
use sui_types::storage::BackingStore;
use sui_types::sui_system_state::{get_sui_system_state_wrapper, AdvanceEpochParams};
use sui_types::type_resolver::LayoutResolver;
use sui_types::{
    base_types::{
        ObjectDigest, ObjectID, ObjectRef, SequenceNumber, SuiAddress, TransactionDigest,
    },
    error::{ExecutionError, SuiError, SuiResult},
    fp_bail,
    gas::GasCostSummary,
    object::Owner,
    object::{Data, Object},
    storage::{BackingPackageStore, ChildObjectResolver, ParentSync, Storage},
    transaction::InputObjects,
};
use sui_types::{is_system_package, SUI_SYSTEM_STATE_OBJECT_ID};

pub struct TemporaryStore<'backing> {
    // The backing store for retrieving Move packages onchain.
    // When executing a Move call, the dependent packages are not going to be
    // in the input objects. They will be fetched from the backing store.
    // Also used for fetching the backing parent_sync to get the last known version for wrapped
    // objects
    store: Arc<dyn BackingStore + Send + Sync + 'backing>,
    tx_digest: TransactionDigest,
    input_objects: BTreeMap<ObjectID, Object>,
    /// The version to assign to all objects written by the transaction using this store.
    lamport_timestamp: SequenceNumber,
    mutable_input_refs: BTreeMap<ObjectID, VersionDigest>, // Inputs that are mutable
    execution_results: ExecutionResultsV2,
    /// Child objects loaded during dynamic field opers
    /// Currently onply populated for full nodes, not for validators
    loaded_child_objects: BTreeMap<ObjectID, LoadedChildObjectMetadata>,
    protocol_config: ProtocolConfig,

    /// Every package that was loaded from DB store during execution.
    /// These packages were not previously loaded into the temporary store.
    runtime_packages_loaded_from_db: RwLock<BTreeMap<ObjectID, Object>>,
}

impl<'backing> TemporaryStore<'backing> {
    /// Creates a new store associated with an authority store, and populates it with
    /// initial objects.
    pub fn new(
        store: Arc<dyn BackingStore + Send + Sync + 'backing>,
        input_objects: InputObjects,
        tx_digest: TransactionDigest,
        protocol_config: &ProtocolConfig,
    ) -> Self {
        let mutable_input_refs = input_objects.mutable_inputs();
        let lamport_timestamp = input_objects.lamport_timestamp();
        let objects = input_objects.into_object_map();
        Self {
            store,
            tx_digest,
            input_objects: objects,
            lamport_timestamp,
            mutable_input_refs,
            execution_results: ExecutionResultsV2::default(),
            protocol_config: protocol_config.clone(),
            loaded_child_objects: BTreeMap::new(),
            runtime_packages_loaded_from_db: RwLock::new(BTreeMap::new()),
        }
    }

    /// WARNING! Should only be used for dry run and dev inspect!
    /// In dry run and dev inspect, you might load a dynamic field that is actually too new for
    /// the transaction. Ideally, we would want to load the "correct" dynamic fields, but as that
    /// is not easily determined, we instead set the lamport version MAX, which is a valid lamport
    /// version for any object used in the transaction (preventing internal assertions or
    /// invariant violations from being triggered)
    pub fn new_for_mock_transaction(
        store: Arc<dyn BackingStore + Send + Sync + 'backing>,
        input_objects: InputObjects,
        tx_digest: TransactionDigest,
        protocol_config: &ProtocolConfig,
    ) -> Self {
        let mutable_input_refs = input_objects.mutable_inputs();
        let lamport_timestamp = SequenceNumber::MAX;
        let objects = input_objects.into_object_map();
        Self {
            store,
            tx_digest,
            input_objects: objects,
            lamport_timestamp,
            mutable_input_refs,
            execution_results: ExecutionResultsV2::default(),
            protocol_config: protocol_config.clone(),
            loaded_child_objects: BTreeMap::new(),
            runtime_packages_loaded_from_db: RwLock::new(BTreeMap::new()),
        }
    }

    // Helpers to access private fields
    pub fn objects(&self) -> &BTreeMap<ObjectID, Object> {
        &self.input_objects
    }

    pub fn update_object_version_and_prev_tx(&mut self) {
        self.execution_results
            .update_version_and_previous_tx(self.lamport_timestamp, self.tx_digest);

        #[cfg(debug_assertions)]
        {
            self.check_invariants();
        }
    }

    /// Break up the structure and return its internal stores (objects, active_inputs, written, deleted)
    pub fn into_inner(self) -> InnerTemporaryStore {
        let results = self.execution_results;
        InnerTemporaryStore {
            input_objects: self.input_objects,
            mutable_inputs: self.mutable_input_refs,
            written: results.written_objects,
            events: TransactionEvents {
                data: results.user_events,
            },
            max_binary_format_version: self.protocol_config.move_binary_format_version(),
            loaded_child_objects: self
                .loaded_child_objects
                .into_iter()
                .map(|(id, metadata)| (id, metadata.version))
                .collect(),
            no_extraneous_module_bytes: self.protocol_config.no_extraneous_module_bytes(),
            runtime_packages_loaded_from_db: self.runtime_packages_loaded_from_db.read().clone(),
        }
    }

    /// For every object from active_inputs (i.e. all mutable objects), if they are not
    /// mutated during the transaction execution, force mutating them by incrementing the
    /// sequence number. This is required to achieve safety.
    pub(crate) fn ensure_active_inputs_mutated(&mut self) {
        let mut to_be_updated = vec![];
        for id in self.mutable_input_refs.keys() {
            if !self.execution_results.objects_modified_at.contains_key(id) {
                // We cannot update here but have to push to `to_be_updated` and update later
                // because the for loop is holding a reference to `self`, and calling
                // `self.write_object` requires a mutable reference to `self`.
                to_be_updated.push(self.input_objects[id].clone());
            }
        }
        for object in to_be_updated {
            // The object must be mutated as it was present in the input objects
            self.mutate_input_object(object);
        }
    }

    pub fn into_effects(
        mut self,
        shared_object_refs: Vec<ObjectRef>,
        transaction_digest: &TransactionDigest,
        transaction_dependencies: Vec<TransactionDigest>,
        gas_cost_summary: GasCostSummary,
        status: ExecutionStatus,
        gas_charger: &mut GasCharger,
        epoch: EpochId,
    ) -> (InnerTemporaryStore, TransactionEffects) {
        self.update_object_version_and_prev_tx();

        // In the case of special transactions that don't require a gas object,
        // we don't really care about the effects to gas, just use the input for it.
        // Gas coins are guaranteed to be at least size 1 and if more than 1
        // the first coin is where all the others are merged.
        let updated_gas_object_info = if let Some(coin_id) = gas_charger.gas_coin() {
            let object = &self.execution_results.written_objects[&coin_id];
            (object.compute_object_reference(), object.owner)
        } else {
            (
                (ObjectID::ZERO, SequenceNumber::default(), ObjectDigest::MIN),
                Owner::AddressOwner(SuiAddress::default()),
            )
        };

        let object_changes = self.execution_results.get_object_changes();

        let lamport_version = self.lamport_timestamp;
        let protocol_version = self.protocol_config.version;
        let inner = self.into_inner();

        let effects = TransactionEffects::new_from_execution_v2(
            protocol_version,
            status,
            epoch,
            gas_cost_summary,
            // TODO: Get rid of shared_objects here.
            shared_object_refs,
            *transaction_digest,
            lamport_version,
            object_changes,
            updated_gas_object_info,
            if inner.events.data.is_empty() {
                None
            } else {
                Some(inner.events.digest())
            },
            transaction_dependencies,
        );
        (inner, effects)
    }

    /// An internal check of the invariants (will only fire in debug)
    #[cfg(debug_assertions)]
    fn check_invariants(&self) {
        // Check not both deleted and written
        debug_assert!(
            {
                self.execution_results
                    .written_objects
                    .keys()
                    .all(|id| !self.execution_results.deleted_object_ids.contains(id))
            },
            "Object both written and deleted."
        );

        // Check all mutable inputs are modified
        debug_assert!(
            {
                self.mutable_input_refs
                    .keys()
                    .all(|id| self.execution_results.objects_modified_at.contains_key(id))
            },
            "Mutable input not modified."
        );

        debug_assert!(
            {
                self.execution_results
                    .written_objects
                    .values()
                    .all(|obj| obj.previous_transaction == self.tx_digest)
            },
            "Object previous transaction not properly set",
        );
    }

    /// Mutate a mutable input object. This is used to mutate input objects outside of Move execution.
    pub fn mutate_input_object(&mut self, object: Object) {
        let id = object.id();
        let input_object = self.mutable_input_refs.get(&id).unwrap();
        self.execution_results
            .objects_modified_at
            .insert(id, *input_object);
        self.execution_results.written_objects.insert(id, object);
    }

    /// Mutate a child object outside of Move. This should be used extremely rarely.
    /// Currently it's only used by advance_epoch_safe_mode because it's all native
    /// without Move. Please don't use this unless you know what you are doing.
    pub fn mutate_child_object(&mut self, old_object: Object, new_object: Object) {
        let id = new_object.id();
        let old_ref = old_object.compute_object_reference();
        debug_assert_eq!(old_ref.0, id);
        self.loaded_child_objects.insert(
            id,
            LoadedChildObjectMetadata {
                version: old_ref.1,
                digest: old_ref.2,
                storage_rebate: old_object.storage_rebate,
            },
        );
        self.execution_results
            .objects_modified_at
            .insert(id, (old_object.version(), old_object.digest()));
        self.execution_results
            .written_objects
            .insert(id, new_object);
    }

    pub fn upgrade_system_package(&mut self, package: Object) {
        let id = package.id();
        debug_assert!(package.is_package());
        let old_obj = self
            .store
            .get_object(&id)
            .unwrap()
            .expect("When upgrading a system package, the current version must exists");
        self.execution_results
            .objects_modified_at
            .insert(id, (old_obj.version(), old_obj.digest()));
        self.execution_results.written_objects.insert(id, package);
    }

    /// Crate a new objcet. This is used to create objects outside of Move execution.
    pub fn create_object(&mut self, object: Object) {
        // Created mutable objects' versions are set to the store's lamport timestamp when it is
        // committed to effects. Creating an object at a non-zero version risks violating the
        // lamport timestamp invariant (that a transaction's lamport timestamp is strictly greater
        // than all versions witnessed by the transaction).
        debug_assert!(
            object.is_immutable() || object.version() == SequenceNumber::MIN,
            "Created mutable objects should not have a version set",
        );
        let id = object.id();
        self.execution_results.created_object_ids.insert(id);
        self.execution_results.written_objects.insert(id, object);
    }

    /// Delete a mutable input object. This is used to delete input objects outside of Move execution.
    pub fn delete_input_object(&mut self, id: &ObjectID) {
        // there should be no deletion after write
        debug_assert!(!self.execution_results.written_objects.contains_key(id));

        let input_object = self.mutable_input_refs.get(id).unwrap();
        self.execution_results
            .objects_modified_at
            .insert(*id, *input_object);
        self.execution_results.deleted_object_ids.insert(*id);
    }

    pub fn drop_writes(&mut self) {
        self.execution_results.drop_writes();
    }

    pub fn read_object(&self, id: &ObjectID) -> Option<&Object> {
        // there should be no read after delete
        debug_assert!(!self.execution_results.deleted_object_ids.contains(id));
        self.execution_results
            .written_objects
            .get(id)
            .or_else(|| self.input_objects.get(id))
    }

    pub fn save_loaded_child_objects(
        &mut self,
        loaded_child_objects: BTreeMap<ObjectID, LoadedChildObjectMetadata>,
    ) {
        #[cfg(debug_assertions)]
        {
            for (id, v1) in &loaded_child_objects {
                if let Some(v2) = self.loaded_child_objects.get(id) {
                    assert_eq!(v1, v2);
                }
            }
            for (id, v1) in &self.loaded_child_objects {
                if let Some(v2) = loaded_child_objects.get(id) {
                    assert_eq!(v1, v2);
                }
            }
        }
        // Merge the two maps because we may be calling the execution engine more than once
        // (e.g. in advance epoch transaction, where we may be publishing a new system package).
        self.loaded_child_objects.extend(loaded_child_objects);
    }

    // TODO: Simplify this logic for effects v2.
    pub fn estimate_effects_size_upperbound(&self) -> usize {
        let num_deletes = self.execution_results.deleted_object_ids.len()
            + self
                .execution_results
                .objects_modified_at
                .keys()
                .filter(|id| {
                    // Filter for wrapped objects.
                    !self.execution_results.written_objects.contains_key(id)
                        && !self.execution_results.deleted_object_ids.contains(id)
                })
                .count();
        // In the worst case, the number of deps is equal to the number of input objects
        TransactionEffects::estimate_effects_size_upperbound(
            self.execution_results.written_objects.len(),
            self.mutable_input_refs.len(),
            num_deletes,
            self.input_objects.len(),
        )
    }

    pub fn written_objects_size(&self) -> usize {
        self.execution_results
            .written_objects
            .values()
            .fold(0, |sum, obj| sum + obj.object_size_for_gas_metering())
    }

    /// If there are unmetered storage rebate (due to system transaction), we put them into
    /// the storage rebate of 0x5 object.
    /// TODO: This will not work for potential future new system transactions if 0x5 is not in the input.
    /// We should fix this.
    pub fn conserve_unmetered_storage_rebate(&mut self, unmetered_storage_rebate: u64) {
        if unmetered_storage_rebate == 0 {
            // If unmetered_storage_rebate is 0, we are most likely executing the genesis transaction.
            // And in that case we cannot mutate the 0x5 object because it's newly created.
            // And there is no storage rebate that needs distribution anyway.
            return;
        }
        tracing::debug!(
            "Amount of unmetered storage rebate from system tx: {:?}",
            unmetered_storage_rebate
        );
        let mut system_state_wrapper = self
            .read_object(&SUI_SYSTEM_STATE_OBJECT_ID)
            .expect("0x5 object must be muated in system tx with unmetered storage rebate")
            .clone();
        // In unmetered execution, storage_rebate field of mutated object must be 0.
        // If not, we would be dropping SUI on the floor by overriding it.
        assert_eq!(system_state_wrapper.storage_rebate, 0);
        system_state_wrapper.storage_rebate = unmetered_storage_rebate;
        self.mutate_input_object(system_state_wrapper);
    }
}

impl<'backing> TemporaryStore<'backing> {
    /// returns lists of (objects whose owner we must authenticate, objects whose owner has already been authenticated)
    fn get_objects_to_authenticate(
        &self,
        sender: &SuiAddress,
        gas_charger: &mut GasCharger,
        is_epoch_change: bool,
    ) -> SuiResult<(Vec<ObjectID>, HashSet<ObjectID>)> {
        let gas_objs: HashSet<&ObjectID> = gas_charger.gas_coins().iter().map(|g| &g.0).collect();
        let mut objs_to_authenticate = Vec::new();
        let mut authenticated_objs = HashSet::new();
        for (id, obj) in &self.input_objects {
            if gas_objs.contains(id) {
                // gas could be owned by either the sender (common case) or sponsor (if this is a sponsored tx,
                // which we do not know inside this function).
                // either way, no object ownership chain should be rooted in a gas object
                // thus, consider object authenticated, but don't add it to authenticated_objs
                continue;
            }
            match &obj.owner {
                Owner::AddressOwner(a) => {
                    assert!(sender == a, "Input object not owned by sender");
                    authenticated_objs.insert(*id);
                }
                Owner::Shared { .. } => {
                    authenticated_objs.insert(*id);
                }
                Owner::Immutable => {
                    // object is authenticated, but it cannot own other objects,
                    // so we should not add it to `authenticated_objs`
                    // However, we would definitely want to add immutable objects
                    // to the set of autehnticated roots if we were doing runtime
                    // checks inside the VM instead of after-the-fact in the temporary
                    // store. Here, we choose not to add them because this will catch a
                    // bug where we mutate or delete an object that belongs to an immutable
                    // object (though it will show up somewhat opaquely as an authentication
                    // failure), whereas adding the immutable object to the roots will prevent
                    // us from catching this.
                }
                Owner::ObjectOwner(_parent) => {
                    unreachable!("Input objects must be address owned, shared, or immutable")
                }
            }
        }

        for id in self.execution_results.objects_modified_at.keys() {
            if authenticated_objs.contains(id) || gas_objs.contains(id) {
                continue;
            }
            let old_obj = self.store.get_object(id)?.unwrap_or_else(|| {
                panic!("Modified object must exist in the store: ID = {:?}", id)
            });
            match &old_obj.owner {
                Owner::ObjectOwner(_parent) => {
                    objs_to_authenticate.push(*id);
                }
                Owner::AddressOwner(_) | Owner::Shared { .. } => {
                    unreachable!("Should already be in authenticated_objs")
                }
                Owner::Immutable => {
                    assert!(is_epoch_change, "Immutable objects cannot be written, except for Sui Framework/Move stdlib upgrades at epoch change boundaries");
                    // Note: this assumes that the only immutable objects an epoch change tx can update are system packages,
                    // but in principle we could allow others.
                    assert!(
                        is_system_package(*id),
                        "Only system packages can be upgraded"
                    );
                }
            }
        }
        Ok((objs_to_authenticate, authenticated_objs))
    }

    // check that every object read is owned directly or indirectly by sender, sponsor, or a shared object input
    pub fn check_ownership_invariants(
        &self,
        sender: &SuiAddress,
        gas_charger: &mut GasCharger,
        is_epoch_change: bool,
    ) -> SuiResult<()> {
        let (mut objects_to_authenticate, mut authenticated_objects) =
            self.get_objects_to_authenticate(sender, gas_charger, is_epoch_change)?;

        // Map from an ObjectID to the ObjectID that covers it.
        let mut covered = BTreeMap::new();
        while let Some(to_authenticate) = objects_to_authenticate.pop() {
            let Some(old_obj) = self.store.get_object(&to_authenticate)? else {
                // lookup failure is expected when the parent is an "object-less" UID (e.g., the ID of a table or bag)
                // we cannot distinguish this case from an actual authentication failure, so continue
                continue;
            };
            let parent = match &old_obj.owner {
                Owner::ObjectOwner(parent) => ObjectID::from(*parent),
                owner => panic!(
                    "Unauthenticated root at {to_authenticate:?} with owner {owner:?}\n\
             Potentially covering objects in: {covered:#?}",
                ),
            };

            if authenticated_objects.contains(&parent) {
                authenticated_objects.insert(to_authenticate);
            } else if !covered.contains_key(&parent) {
                objects_to_authenticate.push(parent);
            }

            covered.insert(to_authenticate, parent);
        }
        Ok(())
    }
}

impl<'backing> TemporaryStore<'backing> {
    /// Return the storage rebate of object `id`
    fn get_input_storage_rebate(&self, id: &ObjectID, expected_version: SequenceNumber) -> u64 {
        // A mutated object must either be from input object or child object.
        if let Some(old_obj) = self.input_objects.get(id) {
            old_obj.storage_rebate
        } else if let Some(metadata) = self.loaded_child_objects.get(id) {
            debug_assert_eq!(metadata.version, expected_version);
            metadata.storage_rebate
        } else if let Ok(Some(obj)) = self.store.get_object_by_key(id, expected_version) {
            // The only case where an modified input object is not in the input list nor child object,
            // is when we upgrade a system package during epoch change.
            debug_assert!(obj.is_package());
            obj.storage_rebate
        } else {
            // not a lot we can do safely and under this condition everything is broken
            panic!(
                "Looking up storage rebate of mutated object {:?} should not fail",
                id
            )
        }
    }

    /// Track storage gas for each mutable input object (including the gas coin)
    /// and each created object. Compute storage refunds for each deleted object.
    /// Will *not* charge anything, gas status keeps track of storage cost and rebate.
    /// All objects will be updated with their new (current) storage rebate/cost.
    /// `SuiGasStatus` `storage_rebate` and `storage_gas_units` track the transaction
    /// overall storage rebate and cost.
    pub(crate) fn collect_storage_and_rebate(&mut self, gas_charger: &mut GasCharger) {
        // Use two loops because we cannot mut iterate written while calling get_input_storage_rebate.
        let old_storage_rebates: Vec<_> = self
            .execution_results
            .written_objects
            .keys()
            .map(|object_id| {
                if let Some((version, _)) =
                    self.execution_results.objects_modified_at.get(object_id)
                {
                    self.get_input_storage_rebate(object_id, *version)
                } else {
                    0
                }
            })
            .collect();
        for (object, old_storage_rebate) in self
            .execution_results
            .written_objects
            .values_mut()
            .zip(old_storage_rebates)
        {
            // new object size
            let new_object_size = object.object_size_for_gas_metering();
            // track changes and compute the new object `storage_rebate`
            let new_storage_rebate =
                gas_charger.track_storage_mutation(new_object_size, old_storage_rebate);
            object.storage_rebate = new_storage_rebate;
        }

        self.collect_rebate(gas_charger);
    }

    pub(crate) fn collect_rebate(&self, gas_charger: &mut GasCharger) {
        for (object_id, (version, _)) in &self.execution_results.objects_modified_at {
            if self
                .execution_results
                .written_objects
                .contains_key(object_id)
            {
                continue;
            }
            // get and track the deleted object `storage_rebate`
            let storage_rebate = self.get_input_storage_rebate(object_id, *version);
            gas_charger.track_storage_mutation(0, storage_rebate);
        }
    }
}
//==============================================================================
// Charge gas current - end
//==============================================================================

impl<'backing> TemporaryStore<'backing> {
    pub fn advance_epoch_safe_mode(
        &mut self,
        params: &AdvanceEpochParams,
        protocol_config: &ProtocolConfig,
    ) {
        let wrapper = get_sui_system_state_wrapper(self.store.as_object_store())
            .expect("System state wrapper object must exist");
        let (old_object, new_object) =
            wrapper.advance_epoch_safe_mode(params, self.store.as_object_store(), protocol_config);
        self.mutate_child_object(old_object, new_object);
    }
}

type ModifiedObjectInfo<'a> = (ObjectID, Option<(SequenceNumber, u64)>, Option<&'a Object>);

impl<'backing> TemporaryStore<'backing> {
    fn get_input_sui(
        &self,
        id: &ObjectID,
        expected_version: SequenceNumber,
        layout_resolver: &mut impl LayoutResolver,
    ) -> Result<u64, ExecutionError> {
        if let Some(obj) = self.input_objects.get(id) {
            // the assumption here is that if it is in the input objects must be the right one
            if obj.version() != expected_version {
                invariant_violation!(
                    "Version mismatching when resolving input object to check conservation--\
                     expected {}, got {}",
                    expected_version,
                    obj.version(),
                );
            }
            obj.get_total_sui(layout_resolver).map_err(|e| {
                make_invariant_violation!(
                    "Failed looking up input SUI in SUI conservation checking for input with \
                         type {:?}: {e:#?}",
                    obj.struct_tag(),
                )
            })
        } else {
            // not in input objects, must be a dynamic field
            let Ok(Some(obj))= self.store.get_object_by_key(id, expected_version) else {
                invariant_violation!(
                    "Failed looking up dynamic field {id} in SUI conservation checking"
                );
            };
            obj.get_total_sui(layout_resolver).map_err(|e| {
                make_invariant_violation!(
                    "Failed looking up input SUI in SUI conservation checking for type \
                         {:?}: {e:#?}",
                    obj.struct_tag(),
                )
            })
        }
    }

    /// Return the list of all modified objects, for each object, returns
    /// - Object ID,
    /// - Input: If the object existed prior to this transaction, include their version and storage_rebate,
    /// - Output: If a new version of the object is written, include the new object.
    fn get_modified_objects(&self) -> Vec<ModifiedObjectInfo<'_>> {
        self.execution_results
            .objects_modified_at
            .iter()
            .map(|(id, (version, _))| {
                let storage_rebate = self.get_input_storage_rebate(id, *version);
                let output = self.execution_results.written_objects.get(id);
                (*id, Some((*version, storage_rebate)), output)
            })
            .chain(
                self.execution_results
                    .written_objects
                    .iter()
                    .filter_map(|(id, object)| {
                        if self.execution_results.objects_modified_at.contains_key(id) {
                            None
                        } else {
                            Some((*id, None, Some(object)))
                        }
                    }),
            )
            .collect()
    }

    /// Check that this transaction neither creates nor destroys SUI. This should hold for all txes
    /// except the epoch change tx, which mints staking rewards equal to the gas fees burned in the
    /// previous epoch.  Specifically, this checks two key invariants about storage
    /// fees and storage rebate:
    ///
    /// 1. all SUI in storage rebate fields of input objects should flow either to the transaction
    ///    storage rebate, or the transaction non-refundable storage rebate
    /// 2. all SUI charged for storage should flow into the storage rebate field of some output
    ///    object
    ///
    /// This function is intended to be called *after* we have charged for
    /// gas + applied the storage rebate to the gas object, but *before* we
    /// have updated object versions.
    pub fn check_sui_conserved(
        &self,
        simple_conservation_checks: bool,
        gas_summary: &GasCostSummary,
    ) -> Result<(), ExecutionError> {
        if !simple_conservation_checks {
            return Ok(());
        }
        // total amount of SUI in storage rebate of input objects
        let mut total_input_rebate = 0;
        // total amount of SUI in storage rebate of output objects
        let mut total_output_rebate = 0;
        for (_, input, output) in self.get_modified_objects() {
            if let Some((_, storage_rebate)) = input {
                total_input_rebate += storage_rebate;
            }
            if let Some(object) = output {
                total_output_rebate += object.storage_rebate;
            }
        }

        if gas_summary.storage_cost == 0 {
            // this condition is usually true when the transaction went OOG and no
            // gas is left for storage charges.
            // The storage cost has to be there at least for the gas coin which
            // will not be deleted even when going to 0.
            // However if the storage cost is 0 and if there is any object touched
            // or deleted the value in input must be equal to the output plus rebate and
            // non refundable.
            // Rebate and non refundable will be positive when there are object deleted
            // (gas smashing being the primary and possibly only example).
            // A more typical condition is for all storage charges in summary to be 0 and
            // then input and output must be the same value
            if total_input_rebate
                != total_output_rebate
                    + gas_summary.storage_rebate
                    + gas_summary.non_refundable_storage_fee
            {
                return Err(ExecutionError::invariant_violation(format!(
                    "SUI conservation failed -- no storage charges in gas summary \
                        and total storage input rebate {} not equal  \
                        to total storage output rebate {}",
                    total_input_rebate, total_output_rebate,
                )));
            }
        } else {
            // all SUI in storage rebate fields of input objects should flow either to
            // the transaction storage rebate, or the non-refundable storage rebate pool
            if total_input_rebate
                != gas_summary.storage_rebate + gas_summary.non_refundable_storage_fee
            {
                return Err(ExecutionError::invariant_violation(format!(
                    "SUI conservation failed -- {} SUI in storage rebate field of input objects, \
                        {} SUI in tx storage rebate or tx non-refundable storage rebate",
                    total_input_rebate, gas_summary.non_refundable_storage_fee,
                )));
            }

            // all SUI charged for storage should flow into the storage rebate field
            // of some output object
            if gas_summary.storage_cost != total_output_rebate {
                return Err(ExecutionError::invariant_violation(format!(
                    "SUI conservation failed -- {} SUI charged for storage, \
                        {} SUI in storage rebate field of output objects",
                    gas_summary.storage_cost, total_output_rebate
                )));
            }
        }
        Ok(())
    }

    /// Check that this transaction neither creates nor destroys SUI.
    /// This more expensive check will check a third invariant on top of the 2 performed
    /// by `check_sui_conserved` above:
    ///
    /// * all SUI in input objects (including coins etc in the Move part of an object) should flow
    ///    either to an output object, or be burned as part of computation fees or non-refundable
    ///    storage rebate
    ///
    /// This function is intended to be called *after* we have charged for gas + applied the
    /// storage rebate to the gas object, but *before* we have updated object versions. The
    /// advance epoch transaction would mint `epoch_fees` amount of SUI, and burn `epoch_rebates`
    /// amount of SUI. We need these information for this check.
    pub fn check_sui_conserved_expensive(
        &self,
        gas_summary: &GasCostSummary,
        advance_epoch_gas_summary: Option<(u64, u64)>,
        layout_resolver: &mut impl LayoutResolver,
    ) -> Result<(), ExecutionError> {
        // total amount of SUI in input objects, including both coins and storage rebates
        let mut total_input_sui = 0;
        // total amount of SUI in output objects, including both coins and storage rebates
        let mut total_output_sui = 0;
        for (id, input, output) in self.get_modified_objects() {
            if let Some((version, _)) = input {
                total_input_sui += self.get_input_sui(&id, version, layout_resolver)?;
            }
            if let Some(object) = output {
                total_output_sui += object.get_total_sui(layout_resolver).map_err(|e| {
                    make_invariant_violation!(
                        "Failed looking up output SUI in SUI conservation checking for \
                         mutated type {:?}: {e:#?}",
                        object.struct_tag(),
                    )
                })?;
            }
        }
        // note: storage_cost flows into the storage_rebate field of the output objects, which is
        // why it is not accounted for here.
        // similarly, all of the storage_rebate *except* the storage_fund_rebate_inflow
        // gets credited to the gas coin both computation costs and storage rebate inflow are
        total_output_sui += gas_summary.computation_cost + gas_summary.non_refundable_storage_fee;
        if let Some((epoch_fees, epoch_rebates)) = advance_epoch_gas_summary {
            total_input_sui += epoch_fees;
            total_output_sui += epoch_rebates;
        }
        if total_input_sui != total_output_sui {
            return Err(ExecutionError::invariant_violation(format!(
                "SUI conservation failed: input={}, output={}, \
                    this transaction either mints or burns SUI",
                total_input_sui, total_output_sui,
            )));
        }
        Ok(())
    }
}

impl<'backing> ChildObjectResolver for TemporaryStore<'backing> {
    fn read_child_object(
        &self,
        parent: &ObjectID,
        child: &ObjectID,
        child_version_upper_bound: SequenceNumber,
    ) -> SuiResult<Option<Object>> {
        let obj_opt = self.execution_results.written_objects.get(child);
        if obj_opt.is_some() {
            Ok(obj_opt.cloned())
        } else {
            self.store
                .read_child_object(parent, child, child_version_upper_bound)
        }
    }
}

impl<'backing> Storage for TemporaryStore<'backing> {
    fn reset(&mut self) {
        self.drop_writes();
    }

    fn read_object(&self, id: &ObjectID) -> Option<&Object> {
        TemporaryStore::read_object(self, id)
    }

    /// Take execution results v2, and translate it back to be compatible with effects v1.
    fn record_execution_results(&mut self, results: ExecutionResults) {
        let ExecutionResults::V2(results) = results else {
            panic!("ExecutionResults::V2 expected in sui-execution v1 and above");
        };
        // It's important to merge instead of override results because it's
        // possible to execute Move runtime more than once during tx execution.
        self.execution_results.merge_results(results);
    }

    fn save_loaded_child_objects(
        &mut self,
        loaded_child_objects: BTreeMap<ObjectID, LoadedChildObjectMetadata>,
    ) {
        TemporaryStore::save_loaded_child_objects(self, loaded_child_objects)
    }
}

impl<'backing> BackingPackageStore for TemporaryStore<'backing> {
    fn get_package_object(&self, package_id: &ObjectID) -> SuiResult<Option<Object>> {
        if let Some(obj) = self.execution_results.written_objects.get(package_id) {
            Ok(Some(obj.clone()))
        } else {
            self.store.get_package_object(package_id).map(|obj| {
                // Track object but leave unchanged
                if let Some(v) = obj.clone() {
                    // TODO: Can this lock ever block execution?
                    self.runtime_packages_loaded_from_db
                        .write()
                        .insert(*package_id, v);
                }
                obj
            })
        }
    }
}

impl<'backing> ModuleResolver for TemporaryStore<'backing> {
    type Error = SuiError;
    fn get_module(&self, module_id: &ModuleId) -> Result<Option<Vec<u8>>, Self::Error> {
        let package_id = &ObjectID::from(*module_id.address());
        let package_obj;
        let package = match self.read_object(package_id) {
            Some(object) => object,
            None => match self.store.get_package_object(package_id)? {
                Some(object) => {
                    package_obj = object;
                    &package_obj
                }
                None => {
                    return Ok(None);
                }
            },
        };
        match &package.data {
            Data::Package(c) => Ok(c
                .serialized_module_map()
                .get(module_id.name().as_str())
                .cloned()),
            _ => Err(SuiError::BadObjectType {
                error: "Expected module object".to_string(),
            }),
        }
    }
}

impl<'backing> ResourceResolver for TemporaryStore<'backing> {
    type Error = SuiError;

    fn get_resource(
        &self,
        address: &AccountAddress,
        struct_tag: &StructTag,
    ) -> Result<Option<Vec<u8>>, Self::Error> {
        let object = match self.read_object(&ObjectID::from(*address)) {
            Some(x) => x,
            None => match self.read_object(&ObjectID::from(*address)) {
                None => return Ok(None),
                Some(x) => {
                    if !x.is_immutable() {
                        fp_bail!(SuiError::ExecutionInvariantViolation);
                    }
                    x
                }
            },
        };

        match &object.data {
            Data::Move(m) => {
                assert!(
                    m.is_type(struct_tag),
                    "Invariant violation: ill-typed object in storage \
                or bad object request from caller"
                );
                Ok(Some(m.contents().to_vec()))
            }
            other => unimplemented!(
                "Bad object lookup: expected Move object, but got {:?}",
                other
            ),
        }
    }
}

impl<'backing> ParentSync for TemporaryStore<'backing> {
    fn get_latest_parent_entry_ref_deprecated(
        &self,
        _object_id: ObjectID,
    ) -> SuiResult<Option<ObjectRef>> {
        unreachable!("Never called in newer protocol versions")
    }
}

impl<'backing> GetModule for TemporaryStore<'backing> {
    type Error = SuiError;
    type Item = CompiledModule;

    fn get_module_by_id(&self, module_id: &ModuleId) -> Result<Option<Self::Item>, Self::Error> {
        let package_id = &ObjectID::from(*module_id.address());
        if let Some(obj) = self.execution_results.written_objects.get(package_id) {
            Ok(Some(
                obj.data
                    .try_as_package()
                    .expect("Bad object type--expected package")
                    .deserialize_module(
                        &module_id.name().to_owned(),
                        self.protocol_config.move_binary_format_version(),
                        self.protocol_config.no_extraneous_module_bytes(),
                    )?,
            ))
        } else {
            self.store.get_module_by_id(module_id)
        }
    }
}
