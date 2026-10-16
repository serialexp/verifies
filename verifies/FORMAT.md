# `verifies` record format

`#[verifies(...)]` leaves one record per annotated test in the compiled test
binary. `cargo-verifies` reads them back. This file is the contract between the
two, and is versioned so a reader can refuse records it does not understand
instead of misreading them. Any other tool (a test runner reporting the keys
natively, say) can read the same records.

## Section

| Object format | Section                              |
|---------------|--------------------------------------|
| ELF           | `verifies`                           |
| PE/COFF       | `verifies` (8 bytes, the PE maximum) |
| Mach-O        | segment `__DATA`, section `__verifies` |

The section holds records back to back. A linker may insert zero bytes
between records (alignment padding from different object files), so a reader
skips any `0x00` bytes before a record. Records from different object files
appear in no particular order.

Each record is a `#[used]` static declared inside the test function's body,
and the body references it (`core::hint::black_box`). Without that reference,
PE linkers (GNU ld and lld for `*-windows-gnu`) garbage-collect the section
even though the static is `#[used]`. With it, a record is kept exactly as
long as its test function is.

## Record, version 1

| Offset | Size | Field                                         |
|--------|------|-----------------------------------------------|
| 0      | 4    | magic, the bytes `VRFY`                       |
| 4      | 1    | version, `1`                                  |
| 5      | 4    | `n`, the payload length, unsigned little-endian |
| 9      | `n`  | payload                                       |

The payload is UTF-8: the test's path, one `0x00` byte, then the keys
separated by `,`.

- **Path**: `module_path!()` at the test function, `::`, and the function's
  name, for example `my_crate::parser::tests::rejects_empty`. Its first
  segment is the crate being compiled. The rest is the test's name as the
  libtest harness (and nextest) reports it.
- **Keys**: at least one. Each is `REQ-<n>` or `REQ-<n>.<m>`, where `n` and
  `m` are decimal integers from 1 to 2147483647 with no sign and no leading
  zeros. The macro rejects anything else at compile time, and a reader
  rejects anything else as malformed. No key appears twice within one record.

A test with several `#[verifies]` attributes has one record per attribute. A
reader unions their keys.

A reader that meets an unknown version must stop with an error naming it.
The length field exists so that later versions can change the payload. It
does not exist so that a reader can skip records it cannot read.
