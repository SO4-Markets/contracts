'use client';
import { createContext, useContext, useState, useCallback, ReactNode } from 'react';

interface WalletState {
  address:    string | null;
  network:    string | null;
  connecting: boolean;
  error:      string | null;
}

interface WalletContextValue extends WalletState {
  connect:    () => Promise<void>;
  disconnect: () => void;
}

const WalletContext = createContext<WalletContextValue | null>(null);

export function WalletProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<WalletState>({
    address: null, network: null, connecting: false, error: null,
  });

  const connect = useCallback(async () => {
    setState(s => ({ ...s, connecting: true, error: null }));
    try {
      const freighter = (window as any).freighter;
      if (!freighter) throw new Error('Freighter extension not installed');
      const { address } = await freighter.getAddress();
      const { network } = await freighter.getNetwork();
      setState(s => ({ ...s, address, network, connecting: false }));
    } catch (err) {
      setState(s => ({
        ...s,
        connecting: false,
        error: err instanceof Error ? err.message : 'Connection failed',
      }));
    }
  }, []);

  const disconnect = useCallback(() => {
    setState({ address: null, network: null, connecting: false, error: null });
  }, []);

  return (
    <WalletContext.Provider value={{ ...state, connect, disconnect }}>
      {children}
    </WalletContext.Provider>
  );
}

export function useWallet() {
  const ctx = useContext(WalletContext);
  if (!ctx) throw new Error('useWallet must be used inside WalletProvider');
  return ctx;
}
