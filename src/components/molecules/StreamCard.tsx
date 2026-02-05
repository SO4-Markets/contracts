'use client';
import Link from 'next/link';
import styles from './StreamCard.module.css';

export interface StreamCardProps {
  id:            string;
  recipient:     string;
  token:         string;
  ratePerSecond: bigint;
  startTime:     number;
  stopTime:      number;
  withdrawn:     bigint;
  cancelled:     boolean;
}

function truncate(addr: string) {
  return `${addr.slice(0, 5)}...${addr.slice(-4)}`;
}

function formatRate(rate: bigint): string {
  const perDay = rate * 86_400n;
  const xlm    = Number(perDay) / 1e7;
  return `${xlm.toFixed(2)} XLM/day`;
}

function progress(start: number, stop: number): number {
  const now     = Math.floor(Date.now() / 1000);
  const elapsed = Math.max(0, Math.min(now, stop) - start);
  const total   = stop - start;
  return total > 0 ? Math.round((elapsed / total) * 100) : 0;
}

export function StreamCard(props: StreamCardProps) {
  const pct = progress(props.startTime, props.stopTime);

  return (
    <Link href={`/streams/${props.id}`} className={styles.card}>
      <div className={styles.header}>
        <span className={styles.id}>Stream #{props.id}</span>
        <span className={`${styles.badge} ${props.cancelled ? styles.cancelled : styles.active}`}>
          {props.cancelled ? 'Cancelled' : 'Active'}
        </span>
      </div>

      <div className={styles.row}>
        <span className={styles.label}>To</span>
        <span className={styles.value}>{truncate(props.recipient)}</span>
      </div>

      <div className={styles.row}>
        <span className={styles.label}>Rate</span>
        <span className={styles.value}>{formatRate(props.ratePerSecond)}</span>
      </div>

      <div className={styles.progressBar}>
        <div className={styles.progressFill} style={{ width: `${pct}%` }} />
      </div>
      <p className={styles.progressLabel}>{pct}% elapsed</p>
    </Link>
  );
}
