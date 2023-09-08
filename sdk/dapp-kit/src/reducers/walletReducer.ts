// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

import type { WalletWithSuiFeatures, WalletAccount, Wallet } from '@mysten/wallet-standard';
import { assertUnreachable } from '../utils/assertUnreachable.js';

export type WalletState = {
	wallets: WalletWithSuiFeatures[];
	currentWallet: WalletWithSuiFeatures | null;
	accounts: readonly WalletAccount[];
	currentAccount: WalletAccount | null;
	connectionStatus: 'disconnected' | 'connecting' | 'connected';
};

type WalletRegisteredAction = {
	type: 'wallet-registered';
	payload: {
		updatedWallets: WalletWithSuiFeatures[];
	};
};

type WalletUnregisteredAction = {
	type: 'wallet-unregistered';
	payload: {
		updatedWallets: WalletWithSuiFeatures[];
		unregisteredWallet: Wallet;
	};
};

type WalletConnectionStatusUpdatedAction = {
	type: 'wallet-connection-status-updated';
	payload: WalletState['connectionStatus'];
};

type WalletConnectedAction = {
	type: 'wallet-connected';
	payload: {
		wallet: WalletWithSuiFeatures;
		currentAccount: WalletAccount | null;
	};
};

type WalletDisconnectedAction = {
	type: 'wallet-disconnected';
	payload?: never;
};

export type WalletAction =
	| WalletConnectionStatusUpdatedAction
	| WalletConnectedAction
	| WalletDisconnectedAction
	| WalletRegisteredAction
	| WalletUnregisteredAction;

export function walletReducer(state: WalletState, { type, payload }: WalletAction): WalletState {
	switch (type) {
		case 'wallet-registered': {
			return {
				...state,
				wallets: payload.updatedWallets,
			};
		}
		case 'wallet-unregistered': {
			if (state.currentWallet?.name === payload.unregisteredWallet.name) {
				return {
					...state,
					wallets: payload.updatedWallets,
					currentWallet: null,
					accounts: [],
					currentAccount: null,
					connectionStatus: 'disconnected',
				};
			}
			return {
				...state,
				wallets: payload.updatedWallets,
			};
		}
		case 'wallet-connection-status-updated':
			return {
				...state,
				connectionStatus: payload,
			};
		case 'wallet-connected':
			return {
				...state,
				currentWallet: payload.wallet,
				accounts: payload.wallet.accounts,
				currentAccount: payload.currentAccount,
				connectionStatus: 'connected',
			};
		case 'wallet-disconnected': {
			return {
				...state,
				currentWallet: null,
				accounts: [],
				currentAccount: null,
				connectionStatus: 'disconnected',
			};
		}
		default:
			assertUnreachable(type);
	}
}
