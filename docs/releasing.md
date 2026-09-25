# Publishing a release

Guixvis release downloads use ZUPT, not gzip tarballs. The source archive is
named `guixvis-<version>.zupt` and contains one directory,
`guixvis-<version>/`, exported from the exact release commit. It includes the
lockfile, tests, Emacs library, documentation, and tracked screenshots/videos.
It does not include `.git`, build output, local caches, or credentials.

Publish the same archive and `SHA256SUMS` on every configured forge. This is a
source package, not a portable prebuilt executable or an offline Cargo vendor
bundle. Building it needs Rust 1.88+ and the locked crates; running it needs Guix.

## Prepare and check

1. Confirm the version, clean working tree, Git identity, existing remote tags,
   and authenticated account on each forge. Never move a published tag.
2. Run the checks in the README and the [0.7.0 notes](releases/0.7.0.md):
   formatting, all-target/all-feature Clippy, default and all-feature tests,
   both JavaScript suites, live Guix checks, Emacs ERT and warnings-as-errors
   byte compilation, a release build, and `cargo audit`. Exercise terminal
   navigation in a real PTY and browser interactions in isolated Chromium.
   Review the release notes and commit any final changes.
3. Export the committed tree into a fresh staging directory. Use `git archive`
   with `--prefix=guixvis-<version>/`; an uncompressed tar stream can transport
   the Git tree into staging, but is not a published release package.
4. From the staging directory, create the archive with ZUPT level 9, without
   encryption:

   ```sh
   zupt compress -l 9 /path/to/output/guixvis-0.7.0.zupt guixvis-0.7.0
   zupt test /path/to/output/guixvis-0.7.0.zupt
   zupt list /path/to/output/guixvis-0.7.0.zupt
   ```

5. Extract into another fresh directory with `zupt extract -o <directory>`.
   Compare paths, file contents, and executable bits against the committed
   tree. Scan the export for secrets. Stop on any unexpected file or mismatch.
6. In the output directory, create `SHA256SUMS` with
   `sha256sum guixvis-0.7.0.zupt > SHA256SUMS`, then run
   `sha256sum -c SHA256SUMS`.

## Native Guix installation

A local native Guix installation is part of the 0.7.0 delivery. Use the
existing Guix recipe approach, pointing only Guixvis at the exact 0.7.0 source
commit and keeping all recipe tests enabled. A channel's current
`guix install guixvis` may still select an older packaged version; inspect
the recipe and resolved package version before installing. Change only
Guixvis in the existing profile, retaining the previous profile generation
for recovery. Confirm that the `guixvis` found on `PATH` reports 0.7.0,
that `guixvis web` serves the local browser and Emacs API, and that the
installed Emacs library loads. Record the profile generation and results in
the delivery report.

## Publish and verify

Create an annotated `v<version>` tag under the configured owner identity.
Push the branch and tag without force to the four existing remotes. Confirm
both the tag object and its peeled commit agree everywhere.

Create the releases as drafts, attach the `.zupt` archive and `SHA256SUMS`,
then publish them. Use the real release notes, not a generated activity log.
Keep tokens out of arguments, logs, files, and the archive; scope each token to
its own HTTPS host. Verify release authors and download the public assets to
check their SHA-256 hashes against the local files.

Do not upload `.tar.gz` release packages. Forge-generated source links are
controlled by the hosting platform and may still offer zip/tar downloads;
they are not the maintained release assets. Internal Guix recipe inputs are
also separate from these public release packages.
