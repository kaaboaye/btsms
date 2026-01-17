# Agent Development Guidelines

## Critical Rule: Test Everything

**MANDATORY**: Every module needs unit tests. No exceptions.

**Why**: Without tests, the codebase becomes unmaintainable mess.

## Before Committing (CI Checks)

**MANDATORY**: Run all these checks before committing. CI will reject code that fails any of them.

```bash
cargo build                   # Must compile
cargo fmt                     # Format code
cargo clippy                  # No warnings allowed
cargo test                    # All tests must pass
```

Quick one-liner to run all checks:

```bash
cargo build && cargo fmt && cargo clippy && cargo test
```

## Rules

1. Every new function = tests
2. Test edge cases (empty, null, invalid)
3. Mock external deps (no real Bluetooth in unit tests)
4. Clear test names
5. Fast tests (milliseconds)

That's it. Write tests or the code will become unmaintainable.

There cannot be any warnings returned by cargo. If you see any, you MUST fix them. They NEVER can be suppressed. THEY MUST BE FIXED.

ALWAYS use `cargo add` to add new deps. Do not type versions by hand because we wanna be sure, we are on the latest.

## Extra online search CLI

You can always ask questions online with this CLI tool.

```bash
ask-online 'How do I list files in Bash?'
```

## Documentation

Keep `README.md` up to date. When adding new features, CLI commands, or changing existing functionality, update the README accordingly.
