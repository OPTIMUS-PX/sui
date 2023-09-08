// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

import type { UseMutationOptions } from '@tanstack/react-query';
import { useMutation } from '@tanstack/react-query';
import type {
	StandardConnectInput,
	StandardConnectOutput,
	WalletAccount,
} from '@mysten/wallet-standard';
import { useWalletContext } from '../../components/WalletProvider.js';
import { WalletAlreadyConnectedError, WalletNotFoundError } from '../../errors/walletErrors.js';
import {
	getMostRecentWalletConnectionInfo,
	setMostRecentWalletConnectionInfo,
} from 'dapp-kit/src/utils/walletUtils';
import { walletMutationKeys } from '../../constants/walletMutationKeys.js';

type ConnectWalletArgs = {
	/** The name of the wallet as defined by the wallet standard to connect to. */
	walletName: string;
} & StandardConnectInput;

type ConnectWalletResult = StandardConnectOutput;

type UseConnectWalletMutationOptions = Omit<
	UseMutationOptions<ConnectWalletResult, Error, ConnectWalletArgs, unknown>,
	'mutationFn'
>;

/**
 * Mutation hook for establishing a connection to a specific wallet.
 */
export function useConnectWallet({
	mutationKey,
	...mutationOptions
}: UseConnectWalletMutationOptions = {}) {
	const { wallets, currentWallet, storageAdapter, storageKey, dispatch } = useWalletContext();

	return useMutation({
		mutationKey: walletMutationKeys.connectWallet(mutationKey),
		mutationFn: async ({ walletName, ...standardConnectInput }) => {
			if (currentWallet) {
				throw new WalletAlreadyConnectedError(
					currentWallet.name === walletName
						? `The user is already connected to wallet ${walletName}.`
						: "You must disconnect the wallet you're currently connected to before connecting to a new wallet.",
				);
			}

			const wallet = wallets.find((wallet) => wallet.name === walletName);
			if (!wallet) {
				throw new WalletNotFoundError(
					`Failed to connect to wallet with name ${walletName}. Double check that the name provided is correct and that a wallet with that name is registered.`,
				);
			}

			dispatch({ type: 'wallet-connection-status-updated', payload: 'connecting' });

			try {
				const connectResult = await wallet.features['standard:connect'].connect(
					standardConnectInput,
				);

				// When connecting to a wallet, we want to connect to the most recently used wallet account if
				// that information is present. This allows for a more intuitive connection experience!
				const mostRecentConnectionInfo = await getMostRecentWalletConnectionInfo(
					storageAdapter,
					storageKey,
				);
				const selectedAccount = getSelectedAccount(
					walletName,
					connectResult.accounts,
					mostRecentConnectionInfo,
				);

				dispatch({
					type: 'wallet-connected',
					payload: { wallet, currentAccount: selectedAccount },
				});

				await setMostRecentWalletConnectionInfo({
					storageAdapter,
					storageKey,
					walletName,
					accountAddress: selectedAccount?.address,
				});

				return connectResult;
			} catch (error) {
				dispatch({ type: 'wallet-connection-status-updated', payload: 'disconnected' });
				throw error;
			}
		},
		...mutationOptions,
	});
}

function getSelectedAccount(
	walletName: string,
	connectedAccounts: readonly WalletAccount[],
	mostRecentConnectionInfo?: { walletName: string; accountAddress?: string } | null,
) {
	if (connectedAccounts.length === 0) {
		return null;
	}

	const hasRecentlyConnectedWallet = mostRecentConnectionInfo?.walletName === walletName;
	if (hasRecentlyConnectedWallet && mostRecentConnectionInfo.accountAddress) {
		const recentWalletAccount = connectedAccounts.find(
			(account) => account.address === mostRecentConnectionInfo.accountAddress,
		);
		return recentWalletAccount ?? connectedAccounts[0];
	}

	return connectedAccounts[0];
}
