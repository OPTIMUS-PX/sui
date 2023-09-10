// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

import { act, renderHook, waitFor } from '@testing-library/react';
import { createWalletProviderContextWrapper, registerMockWallet } from '../test-utils.js';
import { useConnectWallet, useWallet } from 'dapp-kit/src/index.js';

describe('WalletProvider', () => {
	test('auto-connecting to a wallet works successfully', async () => {
		const { unregister, mockWallet } = registerMockWallet('Mock Wallet 1');

		const wrapper = createWalletProviderContextWrapper({
			autoConnect: true,
		});

		const { result } = renderHook(
			() => ({
				connectWallet: useConnectWallet(),
				walletInfo: useWallet(),
			}),
			{ wrapper },
		);

		result.current.connectWallet.mutate({ wallet: mockWallet });
		await waitFor(() => expect(result.current.connectWallet.isSuccess).toBe(true));

		await waitFor(() => {
			expect(result.current.currentWallet).toBeTruthy();
		});
		expect(result.current.currentWallet!.name).toStrictEqual('Mock Wallet 1');

		act(() => unregister());
	});
});
