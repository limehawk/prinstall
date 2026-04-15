<!--
Thanks for contributing! A few questions below help reviews land faster.
Delete the sections that don't apply to your PR.
-->

## What this changes

<!-- One sentence on the user-visible effect, then more detail if needed. -->

## Kind of change

- [ ] Bug fix
- [ ] New feature
- [ ] Driver data (`data/drivers.toml` / `data/known_matches.toml`)
- [ ] Documentation
- [ ] Refactor / cleanup
- [ ] CI / build / tooling

## Testing

<!-- How did you verify this works? Pick any that apply. -->

- [ ] `cargo test` passes (default build)
- [ ] `cargo test --no-default-features` passes (lean build)
- [ ] `cargo clippy -- -W clippy::all` is clean for the files I touched
- [ ] I tested on a real Windows machine / VM
- [ ] For driver data changes: I actually installed this driver on the model listed

## Windows output (if user-facing)

<details>
<summary>Before</summary>

```text
<!-- paste existing behavior -->
```

</details>

<details>
<summary>After</summary>

```text
<!-- paste new behavior -->
```

</details>

## Checklist

- [ ] Commit message follows the `type: subject` style (`feat:`, `fix:`, `chore:`, `docs:`, `data:`, …)
- [ ] Targeted at `dev` branch (not `main`)
- [ ] No secrets, credentials, or customer-identifying data in the diff
- [ ] If this changes PowerShell calls, they go through `PsExecutor` and have a MockExecutor test
