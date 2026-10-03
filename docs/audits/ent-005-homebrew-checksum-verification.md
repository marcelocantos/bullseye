# ENT-005 / T74.5: Homebrew checksum verification (2026-10-03)

**Conclusion:** The audit's claim that `skip_checksum: true` makes the advertised Homebrew install skip artifact verification is refuted. Do not flip that flag as a checksum fix. This does **not** claim that the release is signed or protected against a publisher who changes both the artifact and the formula digest; signing/provenance is separate scope.

## Shipped formula

Read-only sources: [published tap formula](https://github.com/marcelocantos/homebrew-tap/blob/master/Formula/bullseye.rb) (v0.57.0 when checked; GitHub blob SHA `fe71eed1d2e8492131295b9b8b59a60a1bcabe8b`), [v0.57.0 release](https://github.com/marcelocantos/bullseye/releases/tag/v0.57.0). The formula has a `url` and `sha256` for the GitHub source-tag tarball, plus a `url` and `sha256` for **each** of `darwin-arm64`, `linux-amd64`, and `linux-arm64` release tarballs. In particular, the macOS arm64 digest is `1a5cb5ce52b42db1e3dfb8e681ffa6d71eef20058de66ed3906dcc3a16eb27c0`. The local installed tap formula had that same Git blob hash (`git hash-object /opt/homebrew/Library/Taps/marcelocantos/homebrew-tap/Formula/bullseye.rb`). `brew info --json=v2 marcelocantos/tap/bullseye` selected that macOS URL and digest as stable on this machine.

The old audit [ENT-005](entropy-audit-2026-08-22.md#ent-005-homebrew-release-skips-checksums) cites a removed Actions workflow and conflates Homebrew's formula digests with a separate checksum release artifact. The current [tapper.yaml](../../tapper.yaml) explains that `skip_checksum: true` suppresses homebrew-releaser's upload of `checksum.txt` to the *source release*, not formula `sha256` fields. Independently, [release-package.sh](../../scripts/release-package.sh) writes `bullseye-<version>-SHA256SUMS`, and [release-publish.sh](../../scripts/release-publish.sh) uploads it next to the tarballs. This extra file is not the formula verification mechanism.

## Executable mismatch oracle

Run from this repo on a Mac with Homebrew:

```sh
/Users/marcelo/work/github.com/marcelocantos/jevons/bin/gate -clean -- brew ruby scripts/check-homebrew-checksum.rb
```

The fixture is made in a disposable temporary directory. The script calls the actual Homebrew `Downloadable.verification_cache.verify` API on matching bytes, rewrites *only that fixture*, then demands `ChecksumMismatchError` on the same expected digest; the fixture is removed automatically. No install, tap edit, release download or asset replacement occurs. Homebrew's [`Downloadable#fetch`](https://github.com/Homebrew/brew/blob/master/Library/Homebrew/downloadable.rb) invokes `verify_download_integrity` by default; `VerificationCache#verify` raises that mismatch error. This demonstrates the checksum-rejection mechanism, not a full remote `brew install` journey or source authenticity.

**Disposition for PO:** ENT-005's missing-checksum mechanism is not reproducible on the shipped v0.57.0 path. PO decides whether to set T74.5 aside; no ledger status is changed by this note.
