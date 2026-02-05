'use client';
import { useWallet } from '@/context/WalletContext';
import { StreamCard } from '@/components/molecules/StreamCard';
import styles from './dashboard.module.css';

// Mock data — replaced by live contract calls in production
const MOCK_STREAMS = [
  {
    id: '0', recipient: 'GBOB123456789012345678901234567890123456789012345678WXYZ',
    token: 'native', ratePerSecond: 116n,
    startTime: 1735689600, stopTime: 1738368000,
    withdrawn: 1_000_000n, cancelled: false,
  },
  {
    id: '1', recipient: 'GCAR123456789012345678901234567890123456789012345678WXYZ',
    token: 'native', ratePerSecond: 231n,
    startTime: 1735776000, stopTime: 1743552000,
    withdrawn: 5_000_000n, cancelled: false,
  },
];

export default function Dashboard() {
  const { address, connect } = useWallet();

  if (!address) {
    return (
      <div className={styles.empty}>
        <p>Connect your wallet to view your streams.</p>
        <button className={styles.connectBtn} onClick={connect}>
          Connect Wallet
        </button>
      </div>
    );
  }

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1>My Streams</h1>
        <a href="/create" className={styles.newBtn}>+ New Stream</a>
      </header>

      {MOCK_STREAMS.length === 0 ? (
        <p className={styles.noStreams}>No streams yet. Create your first one!</p>
      ) : (
        <div className={styles.grid}>
          {MOCK_STREAMS.map(s => <StreamCard key={s.id} {...s} />)}
        </div>
      )}
    </div>
  );
}
