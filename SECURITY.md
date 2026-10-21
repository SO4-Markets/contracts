# Security Policy

The SO4 Markets contracts hold user funds. We take vulnerability reports seriously
and ask that you disclose them privately and responsibly.

## Supported versions

Only the latest commit on `main` and the most recent deployed contract versions
are supported with security fixes.

## Reporting a vulnerability

**Do not open a public GitHub issue, pull request, or discussion for a security
vulnerability.**

Report privately using either channel:

1. **GitHub private vulnerability reporting** — use the
   ["Report a vulnerability"](../../security/advisories/new) button on the
   repository's *Security* tab (preferred).
2. Or contact the maintainers directly through the SO4 Markets organization
   and ask for a private channel before sharing any details.

Please include:

- The affected contract(s)/function(s) and commit or deployment
- A description of the impact (e.g. fund loss, fund lock, privilege escalation, oracle manipulation)
- Steps to reproduce or a proof-of-concept (a failing test is ideal)
- Any suggested mitigation

## What to expect

| Step | Target |
|---|---|
| Acknowledgement of your report | within 3 business days |
| Initial assessment and severity triage | within 7 days |
| Fix or mitigation plan communicated | within 30 days for high/critical |

We will keep you informed of progress and credit you in the advisory unless you
prefer to stay anonymous.

## Scope

In scope: the Soroban contracts under `contracts/` and shared libraries under `libs/`.

Out of scope: third-party dependencies (report upstream), issues requiring
compromised admin/keeper keys, and denial-of-service via ordinary network congestion.

## Safe harbour

Good-faith research that follows this policy, avoids privacy violations and
service disruption, and does not exploit a finding beyond what is needed to
demonstrate it will not be pursued by the maintainers.
