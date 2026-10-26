# AGENTS.md

narwal-cli: a Rust CLI that publishes Nix store paths to Blossom servers and
announces the cache state on Nostr. A Narwal index server subscribes to the
root events, indexes the manifest, and serves the whole thing to Nix clients
as a regular binary cache.

Pure Rust — no `nix`, `nix-store`, or daemon dependency. All Nix-format work
(NAR encoding, hashing, reference scanning, narinfo build/sign, store-path
types) is done by the Cachix crates: `nix-archive`, `nix-narinfo`,
`nix-derivation`.

## Commands

```bash
nix develop        # rustc/cargo/clippy/rustfmt + liblzma/libzstd
cargo test         # unit tests, no network required
cargo fmt           # formatting
cargo clippy        # lint
nix build .#default   # release binary
```

Test a publish end-to-end without touching production:

```bash
narwal-cli /nix/store/<hash>-hello-2.12.1 \
  --sec-file ./nostr.sec \
  --blossom http://<blossom-host>:3000 \
  --relay ws://<relay-host>:32847 \
  --channel test \
  --nix-sig-key ./mycache.sec \
  --dry-run   # builds + hashes everything, uploads/publishes nothing
```

## How the publish pipeline works

`lib.rs::publish()` runs the pipeline; `main.rs` is a thin clap wrapper.
Per store path (closure resolved first, dependencies-first order):

1. Encode NAR, compute `NarHash`/`NarSize`, scan references.
2. Compress (xz/zstd) → `FileHash`/`FileSize`, build + sign narinfo.
3. Upload `.nar` + `.narinfo` blobs to Blossom.
4. Merge entries into the existing manifest tree, rebuild, upload changed
   nodes, publish the hashtree root pointer to Nostr.

Key property: the root event is published only after every blob is uploaded
(atomic update). Incremental publishes fetch the previous tree, merge, and
republish; unchanged blob hashes are skipped naturally since content is
addressed.

## Module map

- `lib.rs` — pipeline orchestration; entry point `publish()`.
- `main.rs` — CLI (clap): flags, `pair` subcommand, publish-time validation.
- `closure.rs` — closure resolution via reference scanning. Candidate set
  is a readdir of `/nix/store`; references are hashparts found in NAR bytes.
- `nar.rs` — NAR encode + hash (`nix-archive`), byte-identical to
  `nix-store --dump`.
- `refscan.rs` — reference pattern scanning (single pass yields NAR hash,
  size, and references).
- `compress.rs` — xz/zstd compression; FileHash = SHA-256 of compressed
  bytes = Blossom blob address.
- `narinfo.rs` — narinfo assembly via `nix-narinfo` builder (canonical field
  order). Never hand-roll narinfo text.
- `signing.rs` — Ed25519 narinfo signing (`nix keygen-secret` key format).
- `blossom.rs` — blob upload (BUD-02 PUT) with NIP-98 auth, session tokens.
- `blossom_fetch.rs` — blob fetch (BUD-01 GET) for tree reads.
- `manifest.rs` — BUD-16 hashtree Dir tree builder (msgpack nodes,
  174-link chunking). Writes are the wire format consumed by Narwal —
  treat as a compat boundary.
- `tree_reader.rs` — inverse: flatten existing tree back to entries
  (used for incremental merges).
- `nhash.rs` — bech32 `nhash` encoding of the tree root.
- `nostr_pub.rs` — publish root event: kind 17091 (default cache) or
  37091 (named channel), tags `htree`, `blossom`, `nixSigKey`, `d`.
- `nostr_fetch.rs` — fetch latest root event (for incremental updates;
  signer optional — signerless for read-only queries).
- `trust.rs` — consumer-side `trust` subcommand: fetch a cache's root
  event for a pinned npub and print the Nix trust config (`conf`/`env`
  /`json`). Snippet to stdout, guidance to stderr; unsigned caches need
  `--allow-unsigned`. No key material needed.
- `nip46/` — remote Nostr signing: `--bunker` signer, `nostrconnect://` QR
  pairing (`pair` subcommand), pairing session persistence.
- `secrets.rs` — key file loading; refuses group/world-readable files
  (hard error, not a warning).
- `store_path.rs` — StorePath parsing/rendering helpers.

## Domain facts

- Manifest tree entries are `<store-path-hashpart>.narinfo` (plus one
  `nix-cache-info`). The tree contains **no package names** — identity lives
  only inside each narinfo's `StorePath:` field. Indexers that want package
  listings must fetch + parse narinfo blobs (tiny, content-addressed, so
  cacheable by blob hash).
- `NarHash` hashes the uncompressed NAR; `FileHash` hashes the compressed
  file and doubles as the Blossom blob address and narinfo URL component.
- Nostr kind 17091 = default cache, 37091 = named channel. The root event
  points at the hashtree via `htree://nhash...` and lists Blossom servers.
- `nixSigKey` tag values are Nix's own public-key format and are copied
  verbatim into consumers' `trusted-public-keys`. Trust discovery is
  Nostr-anchored: keys read from the publisher's signed event (pinned
  npub) are end-to-end; keys read from a gateway HTTP page just trust the
  gateway operator. The `trust` subcommand implements the former and
  requires `--npub`.
- Key material is **file-only by design**. `--sec`, key args on the command
  line, etc. must never come back — they leak via `ps`, shell history, and
  CI logs. Key files below mode 0600 are refused at load time.
- Nix's `require-sigs` is on by default: input-addressed paths need signed
  narinfos (`--nix-sig-key`) or stock Nix clients reject them.
  Content-addressed paths are self-certifying.

## Rules

- `cargo test` must stay offline and deterministic — no network, no relays.
- Narinfo bytes always go through the `nix-narinfo` builder; fingerprints
  through its `fingerprint()`. No format strings.
- Tree node encoding changes require coordinated updates in Narwal (the
  Elixir index server) — don't alter `WireLink`/`WireTreeNode` casually.
- Don't log secrets, key material, or pairing tokens. Pairing files are
  bearer credentials (written 0600).
- Keep the publish atomicity invariant: never publish a root event before
  all blobs it references are uploaded.
