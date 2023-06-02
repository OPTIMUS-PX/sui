import { Popover } from '@headlessui/react';
import { useGetSystemState } from '@mysten/core';
import { X12 } from '@mysten/icons';
import { formatAddress } from '@mysten/sui.js';
import { useMutation } from '@tanstack/react-query';
import { toast } from 'react-hot-toast';

import Alert from '../../components/alert';
import Logo from '../../components/logo';
import NetworkSelector from '../../components/network-selector';
import { useAppSelector } from '../../hooks';
import { useBackgroundClient } from '../../hooks/useBackgroundClient';
import { Button } from '../../shared/ButtonUI';
import { Link } from '../../shared/Link';
import { CardLayout } from '../../shared/card-layout';
import { Heading } from '../../shared/heading';
import { Text } from '../../shared/text';
import GoogleIcon from '_assets/images/google.svg';
import Twitch from '_assets/images/twitch.svg';

export function ZkPage() {
    const networkName = useAppSelector(({ app: { apiEnv } }) => apiEnv);
    const backgroundClient = useBackgroundClient();
    const { data, isLoading, error, refetch } = useGetSystemState();
    const currentEpoch = data ? Number(data.epoch) : null;
    const createAccountMutation = useMutation({
        mutationKey: ['create', 'zk', 'account'],
        mutationFn: () => {
            if (currentEpoch === null) {
                throw new Error('Missing current epoch');
            }
            return backgroundClient.createZkAccount(currentEpoch);
        },
        onError: (e) =>
            toast.error((e as Error)?.message || 'Error creating account'),
        onSuccess: ({ address, email, pin }) =>
            toast.success(
                (t) => (
                    <div className="flex flex-col gap-1 relative">
                        <X12
                            className="absolute top-0 right-0 cursor-pointer"
                            onClick={() => toast.dismiss(t.id)}
                        />
                        <div className="mb-1">
                            <Heading variant="heading6">
                                Welcome aboard!
                            </Heading>
                        </div>
                        <Text>
                            Account{' '}
                            <span className="font-semibold font-mono">
                                {formatAddress(address)}
                            </span>{' '}
                            was added to your wallet.
                        </Text>
                        <Text variant="bodySmall">
                            To recover your account use your google account{' '}
                            <span className="font-semibold">{email}</span> and
                            the following pin{' '}
                            <span className="font-semibold font-mono">
                                {pin}
                            </span>
                        </Text>
                    </div>
                ),
                { duration: Infinity }
            ),
    });
    return (
        <CardLayout>
            <div className="flex flex-col flex-1 items-center gap-4 self-stretch">
                <Popover className="relative self-stretch flex justify-center">
                    <Popover.Button as="div">
                        <Logo networkName={networkName} />
                    </Popover.Button>
                    <Popover.Panel className="absolute z-10 top-[100%] shadow-lg">
                        <NetworkSelector />
                    </Popover.Panel>
                </Popover>
                <div className="text-center flex flex-col gap-1">
                    <Heading
                        variant="heading4"
                        weight="semibold"
                        color="gray-90"
                    >
                        Welcome to Sui Wallet
                    </Heading>
                    <Text variant="pBody" weight="medium" color="steel-dark">
                        Connecting you to the decentralized web and Sui network.
                    </Text>
                </div>
                <div className="flex-1" />
                <Button
                    variant="outline"
                    text="Sign In with Google"
                    size="tall"
                    loading={createAccountMutation.isLoading || isLoading}
                    disabled={
                        !!(
                            createAccountMutation.isSuccess ||
                            isLoading ||
                            error ||
                            !currentEpoch
                        )
                    }
                    onClick={() => createAccountMutation.mutate()}
                    before={<GoogleIcon />}
                />
                <Button
                    variant="twitch"
                    text="Sign In with Twitch"
                    size="tall"
                    disabled
                    before={<Twitch />}
                />
                <div className="flex-1" />
                {error ? (
                    <div className="self-stretch">
                        <Alert>
                            Error loading current epoch.{' '}
                            <div className="inline-block">
                                <Link
                                    color="suiDark"
                                    text="Retry"
                                    onClick={() => refetch()}
                                    loading={isLoading}
                                />
                            </div>
                        </Alert>
                    </div>
                ) : null}
            </div>
        </CardLayout>
    );
}
