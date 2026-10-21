# Overview

The insurance fund router adds a configurable reserve path for liquidation penalties and shortfall coverage.

It does not replace liquidation accounting directly; it provides a composable helper that can be wired into the current liquidation handler with minimal storage-layout risk.
