# verifies

Trace Rust tests to the requirements they verify, and carry those traces into
the JUnit report [cargo-nextest](https://nexte.st) writes.

```rust
use verifies::verifies;

#[verifies("REQ-142.1", "REQ-142.2")]
#[test]
fn metadata_is_fetched_once_per_ttl() {}
```

```sh
cargo nextest run --profile ci
cargo verifies junit target/nextest/ci/junit.xml -- --profile ci
```

Every test case linked to a key gains a JUnit property, the same convention
Xray and Playwright's JUnit reporter use:

```xml
<testcase name="metadata_is_fetched_once_per_ttl" classname="my_crate">
  <properties>
    <property name="requirements" value="REQ-142.1,REQ-142.2"/>
  </properties>
</testcase>
```

Any tool that reads JUnit can then tell which requirements a run verified,
which failed, and which have no test at all.

## Crates

- **`verifies`**: the `#[verifies(...)]` attribute. It does not change how the
  test runs. It leaves a small record of the test's path and keys in the test
  binary, so a skipped or filtered-out test still reports its keys. Keys are
  checked at compile time. It composes with `#[tokio::test]`, `rstest` and
  `test-case` in either order; for parameterized tests every generated case
  carries the function's keys.
- **`cargo-verifies`**: the `cargo verifies junit <report> -- <nextest args>`
  subcommand. It lists the tests with `cargo nextest list`, reads the records
  out of each test binary, and rewrites the report in place (or to
  `--output`). A linked test that is not in the report is added as skipped.
  Running it twice changes nothing.

Install the subcommand with `cargo install cargo-verifies`, and add the
attribute as a dev-dependency: `cargo add --dev verifies`.

## Keys

A key is `REQ-<n>` for a requirement or `REQ-<n>.<m>` for one of its
acceptance criteria, where `n` and `m` are integers from 1 with no leading
zeros.

## Record format

The records in the test binary are specified in
[`verifies/FORMAT.md`](verifies/FORMAT.md), so other tools (a test runner
reporting the keys natively, say) can read them too. They live in an ELF or
PE section named `verifies`, or `__DATA,__verifies` on Mach-O.

## License

MIT
