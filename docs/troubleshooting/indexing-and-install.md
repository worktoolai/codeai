# Troubleshooting: Indexing and Install

This page covers common failures seen during indexing and local installation.

---

## 1) `codeai index` panic: `byte index ... is not a char boundary`

### Symptom

`codeai index` fails with a panic similar to:

```text
byte index 200 is not a char boundary; it is inside '═' (bytes 199..202)
```

### Cause

A string literal was truncated by byte length at a non-UTF-8 character boundary.

### Fix

Use a build/release that includes UTF-8 boundary-safe truncation in `src/parser.rs`.

### Verify

```bash
codeai index --full --path skills/
```

If the run completes without panic, the issue is resolved.

---

## 2) `./install.sh` fails with `permission denied`

### Symptom

```text
./install.sh: permission denied
```

### Cause

`install.sh` is not executable in your local checkout.

### Fix

Run it with `sh` (project default):

```bash
sh ./install.sh
```

Alternative:

```bash
chmod +x ./install.sh
./install.sh
```

---

## 3) `install.sh` installed an older binary than local source

### Symptom

You patched local source, but installed binary does not include your change.

### Cause

`install.sh` downloads the latest GitHub Release artifact, not your un-released local workspace binary.

### Fix options

- If you need the released artifact: push to `main` and wait for release workflow.
- If you need local test binary immediately: run from local build output (`target/debug/codeai`) during verification.

Project rule for local install workflow is documented in `AGENTS.md`.

---

## 4) Release did not appear immediately after push

### Expected behavior

Releases are automated by GitHub Actions and triggered by push to `main`.

### Checks

1. Confirm your commit is on `origin/main`.
2. Check workflow run status in GitHub Actions.
3. Wait for workflow completion before retrying install.

Do **not** create releases manually with `gh release create` or `gh release upload`.
