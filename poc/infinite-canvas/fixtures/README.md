# Fixtures

The current POC generates deterministic fixtures in code instead of committing large JSON snapshots.

Primary generator:

- `poc/infinite-canvas/web/src/fixtures.ts`

Verification:

```bash
npm run poc:test
```

Use the POC UI `Snapshot` button to export a rendered scene JSON for ad-hoc visual or benchmark comparisons. Promote a generated JSON file into this directory only when it becomes a stable review fixture.
