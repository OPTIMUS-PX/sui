// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

import { act, renderHook, waitFor } from '@testing-library/react';
import { createWalletProviderContextWrapper, registerMockWallet } from '../test-utils.js';
import { useConnectWallet, useWallet } from 'dapp-kit/src/index.js';
import { createMockAccount } from '../mocks/mockAccount.js';

describe('WalletProvider', () => {
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
