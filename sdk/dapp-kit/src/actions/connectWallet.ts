// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

import type {
	StandardConnectInput,
	WalletAccount,
	WalletWithSuiFeatures,
} from '@mysten/wallet-standard';
import type { Dispatch } from 'react';
import type { WalletAction } from '../reducers/walletReducer.js';
import type { StorageAdapter } from '../utils/storageAdapters.js';
import { setMostRecentWalletConnectionInfo } from '../utils/walletUtils.js';

export type ConnectWalletArgs = {
	/** The wallet to connect to. */
	wallet: WalletWithSuiFeatures;

	/** An optional account address to connect to. Defaults to the first authorized account. */
	accountAddress?: string;
} & StandardConnectInput;

export async function connectWallet(
	dispatch: Dispatch<WalletAction>,
	storageAdapter: StorageAdapter,
	storageKey: string,
	{ wallet, accountAddress, ...standardConnectInput }: ConnectWalletArgs,
) {
	dispatch({ type: 'wallet-connection-status-updated', payload: 'connecting' });

	try {
		const connectResult = await wallet.features['standard:connect'].connect(standardConnectInput);
		const selectedAccount = getSelectedAccount(connectResult.accounts, accountAddress);

		dispatch({
			type: 'wallet-connected',
			payload: { wallet, currentAccount: selectedAccount },
		});

		await setMostRecentWalletConnectionInfo({
			storageAdapter,
			storageKey,
			walletName: wallet.name,
			accountAddress: selectedAccount?.address,
		});

		return connectResult;
	} catch (error) {
		dispatch({ type: 'wallet-connection-status-updated', payload: 'disconnected' });
		throw error;
	}
}

function getSelectedAccount(connectedAccounts: readonly WalletAccount[], accountAddress?: string) {
	if (connectedAccounts.length === 0) {
		return null;
	}

	if (accountAddress) {
		const selectedAccount = connectedAccounts.find((account) => account.address === accountAddress);
		return selectedAccount ?? connectedAccounts[0];
	}

	return connectedAccounts[0];
}
