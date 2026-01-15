import Link from 'next/link';
import styles from './page.module.css';

export default function Home() {
  return (
    <main className={styles.main}>
      <section className={styles.hero}>
        <h1 className={styles.title}>
          Real-time payments,<br />second by second.
        </h1>
        <p className={styles.subtitle}>
          SoroProtocol lets you stream tokens continuously on Stellar —
          perfect for salaries, subscriptions, and grants.
        </p>
        <div className={styles.actions}>
          <Link href="/dashboard" className={styles.btnPrimary}>
            Open Dashboard
          </Link>
          <a
            href="https://github.com/SoroProtocol/sdk"
            className={styles.btnSecondary}
            target="_blank"
            rel="noopener noreferrer"
          >
            View SDK
          </a>
        </div>
      </section>

      <section className={styles.features}>
        {FEATURES.map(f => (
          <div key={f.title} className={styles.featureCard}>
            <span className={styles.featureIcon}>{f.icon}</span>
            <h3>{f.title}</h3>
            <p>{f.description}</p>
          </div>
        ))}
      </section>
    </main>
  );
}

const FEATURES = [
  {
    icon: '⚡',
    title: 'Real-time streaming',
    description: 'Tokens flow second-by-second. Recipients can withdraw at any time.',
  },
  {
    icon: '🔒',
    title: 'Non-custodial',
    description: 'Your keys, your funds. Smart contracts hold tokens, not us.',
  },
  {
    icon: '📅',
    title: 'Vesting schedules',
    description: 'Set cliff periods and linear vesting for team grants.',
  },
];
