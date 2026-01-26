'use client';
import { useWallet } from '@/context/WalletContext';
import styles from './WalletButton.module.css';

function truncate(addr: string) {
  return `${addr.slice(0, 5)}...${addr.slice(-4)}`;
}

export function WalletButton() {
  const { address, connecting, connect, disconnect } = useWallet();

  if (address) {
    return (
      <div className={styles.connected}>
        <span className={styles.dot} />
        <span className={styles.addr}>{truncate(address)}</span>
        <button className={styles.disconnect} onClick={disconnect}>
          Disconnect
        </button>
      </div>
    );
  }

  return (
    <button className={styles.connect} onClick={connect} disabled={connecting}>
      {connecting ? 'Connecting…' : 'Connect Wallet'}
    </button>
  );
}
