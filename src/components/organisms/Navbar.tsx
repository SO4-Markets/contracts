'use client';
import Link from 'next/link';
import { usePathname } from 'next/navigation';
import { useState } from 'react';
import { WalletButton } from '@/components/atoms/WalletButton';
import styles from './Navbar.module.css';

const NAV_LINKS = [
  { href: '/dashboard', label: 'Dashboard' },
  { href: '/create',    label: 'New Stream' },
  { href: '/vesting',   label: 'Vesting' },
  { href: '/docs',      label: 'Docs' },
];

export function Navbar() {
  const pathname   = usePathname();
  const [open, setOpen] = useState(false);

  return (
    <nav className={styles.nav}>
      <div className={styles.inner}>
        <Link href="/" className={styles.logo}>
          <span className={styles.logoIcon}>◈</span> SoroProtocol
        </Link>

        <ul className={`${styles.links} ${open ? styles.open : ''}`}>
          {NAV_LINKS.map(l => (
            <li key={l.href}>
              <Link
                href={l.href}
                className={`${styles.link} ${pathname === l.href ? styles.active : ''}`}
                onClick={() => setOpen(false)}
              >
                {l.label}
              </Link>
            </li>
          ))}
        </ul>

        <div className={styles.right}>
          <WalletButton />
          <button
            className={styles.burger}
            onClick={() => setOpen(o => !o)}
            aria-label="Toggle menu"
          >
            <span /><span /><span />
          </button>
        </div>
      </div>
    </nav>
  );
}
