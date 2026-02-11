'use client';
import { useState, FormEvent } from 'react';
import { useWallet } from '@/context/WalletContext';
import styles from './create.module.css';

interface FormState {
  recipient:     string;
  token:         string;
  ratePerDay:    string;
  startDate:     string;
  stopDate:      string;
}

const INITIAL: FormState = {
  recipient: '', token: 'native', ratePerDay: '',
  startDate: '', stopDate: '',
};

export default function CreateStream() {
  const { address } = useWallet();
  const [form,   setForm]   = useState<FormState>(INITIAL);
  const [errors, setErrors] = useState<Partial<FormState>>({});
  const [status, setStatus] = useState<'idle' | 'submitting' | 'done'>('idle');

  function validate(): boolean {
    const e: Partial<FormState> = {};
    if (!form.recipient.match(/^G[A-Z2-7]{55}$/)) e.recipient = 'Invalid Stellar address';
    if (!form.ratePerDay || Number(form.ratePerDay) <= 0) e.ratePerDay = 'Must be > 0';
    if (!form.startDate) e.startDate = 'Required';
    if (!form.stopDate)  e.stopDate  = 'Required';
    if (form.startDate && form.stopDate && form.stopDate <= form.startDate)
      e.stopDate = 'Must be after start date';
    setErrors(e);
    return Object.keys(e).length === 0;
  }

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (!validate()) return;
    setStatus('submitting');
    // Contract call via SDK would go here
    await new Promise(r => setTimeout(r, 1200));
    setStatus('done');
  }

  if (!address) {
    return (
      <div className={styles.gated}>
        <p>Connect your wallet to create a stream.</p>
      </div>
    );
  }

  if (status === 'done') {
    return (
      <div className={styles.success}>
        <span className={styles.checkmark}>✓</span>
        <h2>Stream created!</h2>
        <a href="/dashboard" className={styles.dashLink}>View Dashboard</a>
      </div>
    );
  }

  return (
    <div className={styles.page}>
      <h1 className={styles.title}>New Payment Stream</h1>
      <form className={styles.form} onSubmit={handleSubmit} noValidate>

        <label className={styles.field}>
          <span>Recipient Address</span>
          <input
            className={errors.recipient ? styles.inputError : styles.input}
            placeholder="G..."
            value={form.recipient}
            onChange={e => setForm(f => ({ ...f, recipient: e.target.value }))}
          />
          {errors.recipient && <span className={styles.error}>{errors.recipient}</span>}
        </label>

        <label className={styles.field}>
          <span>Token</span>
          <select
            className={styles.input}
            value={form.token}
            onChange={e => setForm(f => ({ ...f, token: e.target.value }))}
          >
            <option value="native">XLM (Native)</option>
            <option value="usdc">USDC</option>
          </select>
        </label>

        <label className={styles.field}>
          <span>Rate (XLM / day)</span>
          <input
            className={errors.ratePerDay ? styles.inputError : styles.input}
            type="number" min="0" step="0.01"
            placeholder="e.g. 10"
            value={form.ratePerDay}
            onChange={e => setForm(f => ({ ...f, ratePerDay: e.target.value }))}
          />
          {errors.ratePerDay && <span className={styles.error}>{errors.ratePerDay}</span>}
        </label>

        <div className={styles.dateRow}>
          <label className={styles.field}>
            <span>Start Date</span>
            <input
              className={errors.startDate ? styles.inputError : styles.input}
              type="datetime-local"
              value={form.startDate}
              onChange={e => setForm(f => ({ ...f, startDate: e.target.value }))}
            />
            {errors.startDate && <span className={styles.error}>{errors.startDate}</span>}
          </label>
          <label className={styles.field}>
            <span>End Date</span>
            <input
              className={errors.stopDate ? styles.inputError : styles.input}
              type="datetime-local"
              value={form.stopDate}
              onChange={e => setForm(f => ({ ...f, stopDate: e.target.value }))}
            />
            {errors.stopDate && <span className={styles.error}>{errors.stopDate}</span>}
          </label>
        </div>

        <button
          className={styles.submit}
          type="submit"
          disabled={status === 'submitting'}
        >
          {status === 'submitting' ? 'Creating stream…' : 'Create Stream'}
        </button>
      </form>
    </div>
  );
}
