import type { Metadata } from 'next';
import '../styles/globals.css';

export const metadata: Metadata = {
  title:       'SoroProtocol — Payment Streaming on Stellar',
  description: 'Create, manage, and monitor real-time payment streams on the Stellar network.',
  openGraph: {
    title:       'SoroProtocol',
    description: 'Real-time payment streaming on Stellar',
    type:        'website',
  },
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
