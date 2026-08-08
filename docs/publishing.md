# Publishing convention

Two independent implementations of Elrond are being built and compared. They share
**one GitHub repository** and **one Docker Hub image**, separated only by branch
and tag. This document is the contract between them. Both sides must follow it, or
they will overwrite each other's releases.

## This file is the single source of truth

**Canonical location:** `docs/publishing.md` on the **`alt`** branch of
`github.com/tebwritescode/elrond`.

Raw URL, which works without a clone:

```
https://raw.githubusercontent.com/tebwritescode/elrond/alt/docs/publishing.md
```

This is the **only** file the two branches share. Everything else in the two
branches is a separate codebase.

The protocol, for either implementation:

1. **Before any push to GitHub**, read this file from the canonical location. Do
   not rely on a local copy — the other implementation may have updated it.
2. **When the convention changes**, update it *here*, on the `alt` branch, and
   push that change before acting on it. Do not fork a second copy onto `main`;
   two divergent copies of a coordination document is exactly the failure this is
   meant to prevent.
3. If you are working on `main` and need to change the convention, either push the
   edit to the `alt` branch directly, or raise it with the `alt` maintainer. Never
   let `main` carry a conflicting copy.

## The shared coordinates

| | Value |
| --- | --- |
| GitHub repository | `github.com/tebwritescode/elrond` |
| Docker Hub image | `tebwritescode/elrond` |
| Docker Hub secrets | `DOCKERHUB_USERNAME`, `DOCKERHUB_TOKEN` |

## Who owns what

| | Primary implementation | Alt implementation |
| --- | --- | --- |
| Branch | `main` | `alt` |
| Git tags | `v<semver>-beta` | `v<semver>-alt.beta` |
| Docker moving tag | `beta` | `alt-beta` |
| Docker release tag | `<semver>-beta` | `<semver>-alt.beta` |
| Workflow names | `CI`, `Release` | `CI (alt)`, `Release (alt)` |

Example: this build's first release is git tag `v0.1.0-alt.beta`, which publishes
`tebwritescode/elrond:alt-beta` and `tebwritescode/elrond:0.1.0-alt.beta`.

## Rules

**Architectures depend on the ref.** A push to a branch builds `linux/amd64`
only; a version tag builds `linux/amd64,linux/arm64`. arm64 is emulated under
QEMU on hosted runners, and compiling a Rust workspace a second time that way
turns a few minutes into most of an hour — too slow to sit between a push and a
testable image, and unnecessary for a moving beta tag. An official release is
worth the wait and ships everything. Both release workflows express this as

```yaml
platforms: >-
  ${{ startsWith(github.ref, 'refs/tags/')
      && 'linux/amd64,linux/arm64'
      || 'linux/amd64' }}
```

**Neither side publishes `latest`.** Both are beta. When one implementation is
chosen, `latest` is pointed at it deliberately — never as a side effect of a
build. Both release workflows set `flavor: latest=false`.

**Never push to the other side's branch, and never force-push it.** Each
implementation only ever writes to its own branch and its own tags.

**Scope every workflow trigger to your own branch.** A workflow file only runs for
events on the branch it lives on, so `on.push.branches` must name only that
branch. Widening it is what would make the two builds fight over the same Docker
tags.

**Tags must be valid semver with a pre-release suffix.** `v0.1.0-alt.beta` sorts
below `v0.1.0`, so no tooling can mistake a beta for a stable release.
`docker/metadata-action`'s `type=semver` also refuses to derive tags from anything
that is not valid semver.

**Sanitise before pushing.** The repository is public. See the checklist below.

## The pre-push hook enforces this

`.githooks/pre-push` blocks a push that would violate the checklist below. Enable
it once per clone:

```bash
git config core.hooksPath .githooks
```

It lives in the repository rather than in `.git/hooks` so it is version controlled
and applies to everyone who clones, instead of existing on one machine.

Two tiers:

- **Every remote:** credentials (`ghp_`, `github_pat_`, `dckr_pat_`, `AKIA`,
  `xox…`), private-key headers, and personal email addresses. Searched across
  every blob in the commits being pushed, not just the final tree, because a
  credential that was committed and later removed is still published.
- **Public remotes only:** internal hostnames and addresses, and any tracked
  `.env`, database, key, or data directory. These are fine on the self-hosted
  Gitea and are a topology leak on GitHub. Anything that is not the known
  internal host is treated as public, so a newly added public remote is guarded
  by default rather than by being remembered.

`git push --no-verify` bypasses it. If you reach for that, fix the finding
instead.

**The hook ships generic patterns only.** The committed file knows the *shapes*
of private data (RFC 1918 and CGNAT addresses, personal-mailbox domains,
credential prefixes) but never a site-specific literal — the hook is public, so
a real hostname or username written into its patterns would itself be the leak
it exists to prevent. Site-specific literals go in two untracked files that the
hook reads at run time, safely inside `.git/` where they cannot be committed:

```bash
# one extended regex per file; terms joined with |
printf '%s\n' 'my-username|my\.domain\.example' > .git/publish-patterns-identity
printf '%s\n' 'my-hostname|10\.1\.2\.3'         > .git/publish-patterns-internal
```

Recreate them after every fresh clone, alongside `git config core.hooksPath
.githooks`.

## Workflow templates for the primary implementation

Ready to copy onto the `main` branch, already carrying the correct trigger scope
and tag scheme:

- [`docs/templates/main-ci.yml`](templates/main-ci.yml) → `.github/workflows/ci.yml`
- [`docs/templates/main-release.yml`](templates/main-release.yml) → `.github/workflows/release.yml`

The release template is the alt branch's file with four lines changed: name,
trigger, concurrency group, and the moving Docker tag. Keeping the rest identical
is what makes the two implementations' artefacts comparable.

## Sanitisation checklist

Run before the first push and before any release. The pre-push hook checks most
of this automatically.

- No credentials in tracked files or in history. Scan for `ghp_`, `github_pat_`,
  `dckr_pat_`, `AKIA`, `BEGIN … PRIVATE KEY`, and any bearer token shapes.
- No internal hostnames or addresses. The self-hosted Gitea address, the jump
  host, and any Tailscale addresses must not appear in tracked files. A
  `repository = ` field in `Cargo.toml` or `package.json` pointing at the internal
  Gitea instance is the easy one to miss.
- No personal identity. Commits should be authored under the public account name
  with a noreply address, not a personal email.
- `.env`, the data directory, databases, and logs must be ignored, not tracked.
  Only a sanitised `.env.example` is committed, with every secret left empty.
- Docker Hub credentials live in repository or organization secrets, never in a
  workflow file. Only the token is secret; the image name is not, because a masked
  image name turns every log line into `***/***` and makes a failed publish
  impossible to diagnose.

Useful check over the whole history, not just the current tree:

```bash
git log --all -p -G 'ghp_|github_pat_|dckr_pat_|AKIA|BEGIN [A-Z ]*PRIVATE KEY'
git log --all --format='%an <%ae>' | sort -u
```

## Instruction set for the primary implementation

Hand this to whoever maintains the `main` branch build.

> We are publishing both Elrond implementations to one GitHub repository,
> `github.com/tebwritescode/elrond`, and one Docker Hub image,
> `tebwritescode/elrond`. Your build owns the `main` branch; the alternate build
> owns `alt`. Please do the following.
>
> 0. **Read the convention first, every time.** The authoritative copy lives at
>    `docs/publishing.md` on the **`alt`** branch — it is the only file the two
>    branches share, and it may have changed since you last looked:
>
>    ```bash
>    curl -fsSL https://raw.githubusercontent.com/tebwritescode/elrond/alt/docs/publishing.md
>    ```
>
>    If you need to change the convention, edit it on the `alt` branch and push
>    that first. Do not keep a second copy on `main`.
>
> 1. **Sanitise first.** Work through the checklist above. In particular, remove
>    any reference to the internal Gitea address from tracked files, confirm no
>    credentials appear anywhere in history, and confirm commits are authored under
>    a public name with a noreply email. The repository is public.
>
> 2. **Point a remote at the shared repo** and push your build to `main`:
>
>    ```bash
>    git remote add github https://github.com/tebwritescode/elrond.git
>    git push github <your-local-branch>:main
>    ```
>
>    Do not touch the `alt` branch, and do not force-push anything you did not
>    create.
>
> 3. **Copy the two workflow templates** from the `alt` branch. They already carry
>    the correct trigger scope, tag scheme, and Docker tags:
>
>    ```bash
>    git show alt:docs/templates/main-ci.yml      > .github/workflows/ci.yml
>    git show alt:docs/templates/main-release.yml > .github/workflows/release.yml
>    ```
>
>    Adjust the CI job bodies to your toolchain; leave the release workflow alone
>    apart from anything genuinely specific to your build. Two details in it are
>    load-bearing and easy to get wrong: the `secrets` context is **not** available
>    in a step-level `if`, so secret presence is captured in a job-level `env`
>    value; and the published image is scanned **by digest**, not by tag, so the
>    scan provably covers what was just pushed rather than whatever a shared tag
>    points at by then.
>
> 4. **Enable the pre-push hook** so a sanitisation failure is caught before it
>    reaches a public remote:
>
>    ```bash
>    git show alt:.githooks/pre-push > .githooks/pre-push
>    chmod +x .githooks/pre-push
>    git config core.hooksPath .githooks
>    ```
>
> 5. **Confirm the Docker Hub secrets exist** on the repository or organization:
>    `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN`. The token should be a Docker Hub
>    access token with write scope, not an account password.
>
> 6. **Tag your first release** as `v<version>-beta`, for example `v0.1.0-beta`.
>    That publishes `tebwritescode/elrond:beta` and
>    `tebwritescode/elrond:0.1.0-beta`.
>
> Neither implementation publishes `latest` while both are in beta. When one is
> chosen, we point `latest` at it explicitly.

## Promoting a winner

When one implementation is chosen:

1. Add `type=raw,value=latest` to that implementation's release workflow, or
   retag an existing digest:
   `docker buildx imagetools create -t tebwritescode/elrond:latest tebwritescode/elrond:<chosen-tag>`.
2. Drop the `-beta` / `-alt.beta` suffix from its tag scheme and cut a stable
   `vX.Y.Z`.
3. Decide what happens to the other branch: keep it for reference, or remove it.
   Either way, stop its release workflow first so it cannot publish again.
