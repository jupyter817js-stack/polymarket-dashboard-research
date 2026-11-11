# Validation and limitations

Checked: 11 November 2026. These results describe this public source snapshot only.

- Web frontend: npm run typecheck and npm run build passed.
- Next.js updated to 15.5.29 with matching eslint configuration; PostCSS updated to 8.5.32.
- Web frontend npm audit --omit=dev: 0 reported production dependency vulnerabilities. This does not cover development dependencies or Rust dependencies.
- Rust components were not compiled or tested in this environment.
- No database migrations, live trading or external API integration tests were run.

## Publication review

Private operational files and original history are excluded. Source was reviewed for credentials and direct contact information; the new public Git history is scanned with Gitleaks before publication. Narrow inline exemptions identify deterministic test fixtures and public contract addresses, not operational secrets. Automated checks are not a security audit and cannot guarantee the absence of vulnerabilities.
