// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

import { renderHook, waitFor, act } from '@testing-library/react';
import { useConnectWallet, useWallet } from 'dapp-kit/src';
import { createWalletProviderContextWrapper, registerMockWallet } from '../test-utils.js';
import {
	WalletAlreadyConnectedError,
	WalletNotFoundError,
} from 'dapp-kit/src/errors/walletErrors.js';
import type { Mock } from 'vitest';

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
		const { connectWallet, walletInfo } = result.current;

		connectWallet.mutate({ walletName: 'Some random wallet' });

		await waitFor(() => expect(connectWallet.error).toBeInstanceOf(WalletNotFoundError));
		expect(walletInfo.connectionStatus).toBe('disconnected');
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
		const { connectWallet } = result.current;

		connectWallet.mutate({ walletName: 'Mock Wallet 1' });
		await waitFor(() => expect(connectWallet.isSuccess).toBe(true));

		connectWallet.mutate({ walletName: 'Some random wallet' });
		await waitFor(() => expect(connectWallet.error).toBeInstanceOf(WalletAlreadyConnectedError));

		act(() => {
			unregister();
		});
	});

	test('that an error is thrown when a user fails to connect their wallet', async () => {
		const unregister = registerMockWallet('Mock Wallet 1');

		const wrapper = createWalletProviderContextWrapper();
		const { result } = renderHook(
			() => ({
				connectWallet: useConnectWallet(),
				walletInfo: useWallet(),
			}),
			{ wrapper },
		);
		const { connectWallet, walletInfo } = result.current;
		const connectFeature = walletInfo.wallets[0].features['standard:connect'];

		const mockConnect = connectFeature.connect as Mock;
		mockConnect.mockRejectedValueOnce(() => {
			throw new Error('User rejected request');
		});

		connectWallet.mutate({ walletName: 'Mock Wallet 1' });

		await waitFor(() => expect(connectWallet.isError).toBe(true));
		expect(walletInfo.connectionStatus).toBe('disconnected');

		act(() => {
			unregister();
		});
	});

	test('that connecting to a wallet works successfully', async () => {
		const unregister = registerMockWallet('Mock Wallet 1');

		const wrapper = createWalletProviderContextWrapper();
		const { result } = renderHook(
			() => ({
				connectWallet: useConnectWallet(),
				walletInfo: useWallet(),
			}),
			{ wrapper },
		);
		const { connectWallet, walletInfo } = result.current;

		connectWallet.mutate({ walletName: 'Mock Wallet 1' });

		await waitFor(() => expect(connectWallet.isSuccess).toBe(true));
		expect(walletInfo.currentWallet?.name).toBe('Mock Wallet 1');
		expect(walletInfo.accounts).toHaveLength(1);
		expect(walletInfo.currentAccount).not.toBeNull();
		expect(walletInfo.connectionStatus).toBe('connected');

		const savedConnectionInfo = window.localStorage.getItem('sui-dapp-kit:wallet-connection-info');
		expect(savedConnectionInfo).toBeDefined();
		expect(JSON.parse(savedConnectionInfo!)).toStrictEqual({
			walletName: 'Mock Wallet 1',
			accountAddress: walletInfo.currentAccount?.address,
		});

		act(() => {
			unregister();
		});
	});
});
