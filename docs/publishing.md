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

## Sanitisation checklist

Run before the first push and before any release.

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
> 3. **Add a release workflow** at `.github/workflows/release.yml` on your branch,
>    matching the alt branch's file but with these differences:
>
>    - `name: Release`
>    - `on.push.branches: [main]` and `on.push.tags: ['v*-beta']`
>    - `concurrency.group: release-main-${{ github.ref }}`
>    - metadata tags: `type=raw,value=beta` and `type=semver,pattern={{version}}`
>    - `flavor: latest=false` — do **not** publish `latest`
>
>    The alt branch's file is the reference implementation; copy it and change
>    only those lines. Two details in it are load-bearing and easy to get wrong:
>    the `secrets` context is **not** available in a step-level `if`, so capture
>    secret presence in a job-level `env` value and test that instead; and scan the
>    published image **by digest**, not by tag, so you are provably scanning what
>    you just pushed rather than whatever a shared tag now points at.
>
> 4. **Scope your CI workflow** to `branches: [main]` so it does not run on the
>    alt branch's pushes.
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
