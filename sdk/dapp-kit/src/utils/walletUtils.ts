// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

import type { Wallet, WalletWithSuiFeatures } from '@mysten/wallet-standard';
import { isWalletWithSuiFeatures } from '@mysten/wallet-standard';
import type { StorageAdapter } from './storageAdapters.js';

const noSelectedAccountStoragePlaceholder = 'no-selected-account';

export function sortWallets(
	wallets: readonly Wallet[],
	preferredWallets: string[],
	requiredFeatures?: string[],
): WalletWithSuiFeatures[] {
	const suiWallets = wallets.filter((wallet): wallet is WalletWithSuiFeatures =>
		isWalletWithSuiFeatures(wallet, requiredFeatures),
	);

	return [
		// Preferred wallets, in order:
		...(preferredWallets
			.map((name) => suiWallets.find((wallet) => wallet.name === name))
			.filter(Boolean) as WalletWithSuiFeatures[]),

		// Wallets in default order:
		...suiWallets.filter((wallet) => !preferredWallets.includes(wallet.name)),
	];
}

export async function setMostRecentWalletConnectionInfo({
	storageAdapter,
	storageKey,
	walletName,
	accountAddress,
}: {
	storageAdapter: StorageAdapter;
	storageKey: string;
	walletName: string;
	accountAddress?: string;
}) {
	try {
		await storageAdapter.set(
			storageKey,
			`${walletName}-${accountAddress ?? noSelectedAccountStoragePlaceholder}`,
		);
	} catch (error) {
		// We'll skip error handling here and just report the error to the console since persisting connection
		// info isn't essential functionality and storage adapters can be plugged in by the consumer.
		console.error('[dApp-kit] Error: Failed to save wallet connection info to storage.', error);
	}
}

export async function getMostRecentWalletConnectionInfo(
	storageAdapter: StorageAdapter,
	storageKey: string,
) {
	try {
		const lastWalletConnectionInfo = await storageAdapter.get(storageKey);
		if (lastWalletConnectionInfo) {
			const [walletName, accountAddress] = lastWalletConnectionInfo.split('-');
			const isMissingAccountAddress = accountAddress === noSelectedAccountStoragePlaceholder;

			return {
				walletName,
				accountAddress: isMissingAccountAddress ? undefined : accountAddress,
			};
		}
	} catch (error) {
		// We'll skip error handling here and just report the error to the console since retrieving connection
		// info isn't essential functionality and storage adapters can be plugged in by the consumer.
		console.error(
			'[dApp-kit] Error: Failed to retrieve wallet connection info from storage.',
			error,
		);
	}
	return {};
}
