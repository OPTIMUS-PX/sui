// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

import { renderHook, waitFor, act } from '@testing-library/react';
import { useConnectWallet, useWallet } from 'dapp-kit/src';
import { createWalletProviderContextWrapper, registerMockWallet } from '../test-utils.js';
import {
	WalletAlreadyConnectedError,
	WalletNotFoundError,
} from 'dapp-kit/src/errors/walletErrors.js';

describe('useConnectWallet', () => {
	test('that an error is thrown when connecting to a non-existent wallet', async () => {
		const wrapper = createWalletProviderContextWrapper();
		const { result } = renderHook(
			() => ({
				connectWallet: useConnectWallet(),
				walletInfo: useWallet(),
			}),
			{ wrapper },
		);
		result.current.connectWallet.mutate({ walletName: 'Some random wallet' });

		await waitFor(() =>
			expect(result.current.connectWallet.error).toBeInstanceOf(WalletNotFoundError),
		);

		expect(result.current.walletInfo.connectionStatus).toBe('disconnected');
	});

	test('that an error is thrown when connecting to a wallet when a connection is already active', async () => {
		const unregister = registerMockWallet('Mock Wallet 1');

		const wrapper = createWalletProviderContextWrapper();
		const { result } = renderHook(
			() => ({
				connectWallet: useConnectWallet(),
				walletInfo: useWallet(),
			}),
			{ wrapper },
		);
		result.current.connectWallet.mutate({ walletName: 'Mock Wallet 1' });

		await waitFor(() => expect(result.current.connectWallet.isSuccess).toBe(true));

		result.current.connectWallet.mutate({ walletName: 'Some random wallet' });

		await waitFor(() =>
			expect(result.current.connectWallet.error).toBeInstanceOf(WalletAlreadyConnectedError),
		);
		console.log('STOodofoRAGE', window.localStorage);

		act(() => {
			unregister();
		});
	});

	// connect to wallet A with address ABC + DBE
	// remove address ABC from wallet --- A-ABC

	// disconnect
	// go to dapp and connect

	test('that connecting to a wallet works correctly', async () => {
		const unregister = registerMockWallet('Mock Wallet 1');

		const wrapper = createWalletProviderContextWrapper();
		const { result } = renderHook(
			() => ({
				connectWallet: useConnectWallet(),
				walletInfo: useWallet(),
			}),
			{ wrapper },
		);

		result.current.connectWallet.mutate({ walletName: 'Mock Wallet 1' });

		await waitFor(() => expect(result.current.connectWallet.isSuccess).toBe(true));

		expect(result.current.walletInfo.currentWallet?.name).toBe('Mock Wallet 1');
		expect(result.current.walletInfo.accounts).toHaveLength(1);
		expect(result.current.walletInfo.currentAccount).not.toBeNull();
		expect(result.current.walletInfo.connectionStatus).toBe('connected');
		console.log('STORAGE', window.localStorage);
		expect(window.localStorage.getItem('dapp-kit:most-recent-wallet-connection-info')).toBe(
			`Mock Wallet 1-${result.current.walletInfo.currentAccount?.address}`,
		);

		act(() => {
			result.current.walletInfo.currentWallet?.features['standard:disconnect']?.disconnect();
			unregister();
		});
	});
});
