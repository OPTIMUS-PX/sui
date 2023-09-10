// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

import { act, renderHook, waitFor } from '@testing-library/react';
import { createWalletProviderContextWrapper, registerMockWallet } from '../test-utils.js';
import { useConnectWallet, useWallet } from 'dapp-kit/src/index.js';
import { createMockAccount } from '../mocks/mockAccount.js';

describe('WalletProvider', () => {
	test('the correct wallet and account information is returned on initial render', () => {
		const wrapper = createWalletProviderContextWrapper();
		const { result } = renderHook(() => useWallet(), { wrapper });

		expect(result.current).toStrictEqual({
			accounts: [],
			currentAccount: null,
			wallets: [],
			currentWallet: null,
			connectionStatus: 'disconnected',
		});
	});

	test('the list of wallets is ordered correctly by preference', () => {
		const { unregister: unregister1 } = registerMockWallet({ walletName: 'Mock Wallet 1' });
		const { unregister: unregister2 } = registerMockWallet({ walletName: 'Mock Wallet 2' });
		const { unregister: unregister3 } = registerMockWallet({ walletName: 'Mock Wallet 3' });

		const wrapper = createWalletProviderContextWrapper({
			preferredWallets: ['Mock Wallet 2', 'Mock Wallet 1'],
		});
		const { result } = renderHook(() => useWallet(), { wrapper });
		const walletNames = result.current.wallets.map((wallet) => wallet.name);

		expect(walletNames).toStrictEqual(['Mock Wallet 2', 'Mock Wallet 1', 'Mock Wallet 3']);

		act(() => {
			unregister1();
			unregister2();
			unregister3();
		});
	});

	test('the unsafe burner wallet is registered when enableUnsafeBurner is set', async () => {
		const wrapper = createWalletProviderContextWrapper({
			enableUnsafeBurner: true,
		});
		const { result } = renderHook(() => useWallet(), { wrapper });
		const walletNames = result.current.wallets.map((wallet) => wallet.name);

		expect(walletNames).toStrictEqual(['Unsafe Burner Wallet']);
	});

	test('unregistered wallets are removed from the list of wallets', async () => {
		const { unregister: unregister1 } = registerMockWallet({ walletName: 'Mock Wallet 1' });
		const { unregister: unregister2 } = registerMockWallet({ walletName: 'Mock Wallet 2' });
		const { unregister: unregister3 } = registerMockWallet({ walletName: 'Mock Wallet 3' });

		const wrapper = createWalletProviderContextWrapper();
		const { result } = renderHook(() => useWallet(), { wrapper });

		act(() => unregister2());

		const walletNames = result.current.wallets.map((wallet) => wallet.name);
		expect(walletNames).toStrictEqual(['Mock Wallet 1', 'Mock Wallet 3']);

		act(() => {
			unregister1();
			unregister3();
		});
	});

	test('the list of wallets is correctly filtered by required features', () => {
		const { unregister: unregister1 } = registerMockWallet({
			walletName: 'Mock Wallet 1',
			additionalFeatures: {
				'my-dapp:super-cool-feature': {
					version: '1.0.0',
					superCoolFeature: () => {},
				},
			},
		});
		const { unregister: unregister2 } = registerMockWallet({ walletName: 'Mock Wallet 2' });

		const wrapper = createWalletProviderContextWrapper({
			requiredFeatures: ['my-dapp:super-cool-feature'],
		});
		const { result } = renderHook(() => useWallet(), { wrapper });
		const walletNames = result.current.wallets.map((wallet) => wallet.name);

		expect(walletNames).toStrictEqual(['Mock Wallet 1']);

		act(() => {
			unregister1();
			unregister2();
		});
	});

	test('auto-connecting to a wallet works successfully', async () => {
		const { unregister, mockWallet } = registerMockWallet({
			walletName: 'Mock Wallet 1',
			accounts: [createMockAccount(), createMockAccount()],
		});
		const wrapper = createWalletProviderContextWrapper({
			autoConnect: true,
		});

		// Manually connect to the second wallet account so we have a wallet to auto-connect to.
		const { result: connectResult, unmount } = renderHook(() => useConnectWallet(), { wrapper });
		connectResult.current.mutate({
			wallet: mockWallet,
			accountAddress: mockWallet.accounts[1].address,
		});
		await waitFor(() => expect(connectResult.current.isSuccess).toBe(true));

		// Now unmount our component tree to simulate someone leaving the page.
		unmount();

		// Finally, render our component tree again and auto-connect to our previously connected wallet account.
		const { result: walletInfoResult } = renderHook(() => useWallet(), { wrapper });

		await waitFor(() => expect(walletInfoResult.current.currentWallet).toBeTruthy());
		expect(walletInfoResult.current.currentWallet!.name).toStrictEqual('Mock Wallet 1');

		await waitFor(() => expect(walletInfoResult.current.currentAccount).toBeTruthy());
		expect(walletInfoResult.current.currentAccount!.address).toStrictEqual(
			mockWallet.accounts[1].address,
		);

		act(() => unregister());
	});
});
