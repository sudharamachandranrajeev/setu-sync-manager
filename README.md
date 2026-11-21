<div align="center">
  <img src="app/public/favicon.svg" width="140" alt="Setu logo">
  <h1>Setu</h1>
  <h4>Choose exactly which files and folders sync between your computers, and which way they go.</h4>

  <p>
    <img src="https://img.shields.io/badge/status-alpha-orange" alt="Status: alpha">
    <img src="https://img.shields.io/badge/platforms-Windows%20%7C%20Linux-blue" alt="Platforms: Windows and Linux">
    <img src="https://img.shields.io/badge/licence-MIT-green" alt="Licence: MIT">
  </p>

  <p>
    <a href="#-what-it-does">What it does</a> •
    <a href="#-get-started">Get started</a> •
    <a href="#-how-it-works">How it works</a> •
    <a href="#-faq">FAQ</a> •
    <a href="#-for-developers">For developers</a>
  </p>
</div>

Setu is for anyone who keeps one work folder on several computers but does not want *everything*
to go both ways. For example: send all of your laptop's projects to your desktop, but only
let one or two folders come back. Setu shows both computers' files side by side, lets you set a
direction on any folder or file, and shows you exactly what will change before anything does.

Setu is **alpha** software. It does not copy files itself: [Syncthing](https://syncthing.net)
does that, straight from computer to computer over [Tailscale](https://tailscale.com), and Setu
tells Syncthing what to do. Your computers find each other through your **Brahma account**: you
make your choices once, on any computer, and the others set themselves up. It works with up to
eight computers.

> [!WARNING]
> Setu never deletes files when you change a direction or switch a folder off. But Syncthing
> *does* pass deletions along inside a folder that syncs. If you delete a file in a folder set
> to "Both ways", it goes on the other computer too. Syncthing keeps old copies for 30 days in
> each folder's `.stversions` folder, so you can get them back.

## ✨ What it does

- 🧭 **A direction for every folder and file** – *Copy from Windows*, *Copy to Windows*, *Both
  ways* or *Don't sync*, set with one click. Everything inside a folder follows it unless you
  choose otherwise. (Setu names the other computer by its system, so on Windows it says *Linux*.)
- ⚡ **Quick setup in two steps** – pick "copy everything from the other computer", "copy
  everything to it" or "keep both the same", then untick the folders to leave out. Folders are
  listed biggest first, with sizes, so large projects are easy to spot.
- 👀 **Both computers in one view** – see what is here, what is there and what is only on one
  side, with sizes, before you decide.
- 📝 **See before you change** – your choices stay a draft until you open the review, read the
  plan and press **Start syncing**.
- 🔒 **Secrets never travel** – API key folders, `.env` files and signing keys are always left
  out, whatever you choose. Build and cache folders are left out too.
- 🤝 **Choose once, every computer follows** – your choices are shared through your Brahma
  account, locked end to end so only your computers can read them. Each computer's Setu sets
  itself up within a minute or two. Only a change that would replace or remove many files waits
  for you on that computer, and the top bar says which one.
- 🔗 **No pairing by hand** – sign in on a new computer and Setu adds your other computers to
  Syncthing for you, at their Tailscale addresses only. Syncthing's internet discovery and relays
  are switched off.
- 💾 **Backup copies** – mark a computer as a backup and it only receives. Changes made there are
  never sent back.
- 🧮 **Same files everywhere?** – every few hours Setu reads every synced file on each computer
  and compares fingerprints (SHA-256). It tells you when a folder holds exactly the same files on
  all computers, and when not, which files differ.
- 🧹 **No duplicate copies left behind** – a conflict copy or old version that is exactly the same
  as its file is removed, only after three checks: same size, same SHA-256 and every byte
  compared. Your own files are never touched. Every removal is listed and can be put back.
- 📊 **Progress in plain words** – the bottom bar shows what is still copying ("Copying to this
  computer: 1,204 files (3.1 GB) left") and says when everything is copied.
- ↩️ **Nothing lost by changing your mind** – switching a folder off stops syncing it; the files
  stay on both computers.
- 🧱 **Starts with nothing synced** – nothing moves until you choose it.
- 🔍 **File-by-file preview** – before the first sync, Setu compares both computers' file lists
  and shows how many files go each way, which ones get replaced, and which stay on one side.
  If many files would be replaced, it asks a second time.
- 🪟 **Windows name check** – names Windows cannot hold (`CON`, `a:b`, trailing dots, very long
  paths, two names that differ only in case) are listed before anything copies.
- 🛑 **Delete guard** – if more than 50 files are deleted on the other computer within two
  minutes, Setu pauses that folder here and tells you, until you resume it.
- 📶 **Network gate** – syncing waits on a metered connection or a slow relayed link, unless you
  allow it.
- ⚖️ **Conflicts, side by side** – when a file changed on both computers, both copies are kept;
  pick the one you want, and the other is put away, not deleted.
- 🕰️ **Old versions for 30 days** – put back any replaced or deleted file from a list.
- ✅ **Is it all copied?** – one click shows, folder by folder, that every file is complete and
  identical on both computers, or what is still missing and why.
- ⏸️ **Pause** – stop copying on this computer for an hour, until tomorrow morning, or until you
  resume. Changes on either side wait and are copied afterwards.
- 🚫 **Leave out by name** – add a name or pattern, such as `*.iso` or `videos`, and it is never
  copied, in any folder, on either computer.
- 🔔 **Notifications and tray** – a desktop notice when a folder is paused, a conflict appears or
  files cannot sync.

## 🚀 Get started

You need on every computer:

- [Syncthing](https://syncthing.net/downloads/), running.
- [Tailscale](https://tailscale.com/download), signed in to the same tailnet.
- A Brahma account on your own Brahma server (see
  [Brahma](https://github.com/sudharamachandranrajeev/brahma-life-suite)). It only holds your
  choices and each computer's status, encrypted. Your files never go through it.

1. **Install Setu on each computer.** Download it from the
   [Releases page](https://github.com/sudharamachandranrajeev/setu-sync-manager/releases):
   - **Windows:** run `Setu-…-windows-x64-setup.exe`. It is not code-signed yet, so Windows
     SmartScreen may warn: choose **More info**, then **Run anyway**.
   - **Linux:** unpack `setu-…-linux-x64.tar.gz`, open a terminal in that folder and run
     `./install.sh`. It installs for you only (no `sudo`) and adds Setu to the app menu.

   Both start Setu's small background helper at sign-in. It sets up new choices, pairs your
   computers, checks the copies, pauses syncing on a metered network, stops mass deletions and
   shows notifications. Keep it on: without it, the other steps wait until you open Setu.
2. **Open Setu and pick the folder to manage** (for example `/home/you/Work` or `C:\Work`).
3. **Sign in** with your Brahma server address, email and password.
4. **Say how this computer takes part:** *Keep the same files as my other computers*, or *Backup
   copy* (only receives).
5. **Choose what to copy, on any one computer.** Press **Quick setup** for the two-step version,
   or click any folder or file and choose on the right. Then press **Review**, read what will
   happen and press **Start syncing**.
6. **That's it.** Your other computers pick up the choices by themselves. If a change would
   replace or remove many files on one of them, that computer shows a notice and waits for you to
   check it there. The bottom bar shows the progress.

Prefer the terminal? `setu sign-in <server> <email>`, `setu join`, `setu rule …` and
`setu apply` do the same (`setu help` lists everything).

## 🧩 How it works

```mermaid
flowchart LR
  A[Your choices in Setu] --> B[Draft]
  B -->|Review, Start syncing| C[Sync choices, revision N]
  C -->|encrypted rows| S[(Your Brahma server)]
  S --> D[Setu helper on each other computer]
  D -->|safe: applies by itself<br/>big change: waits for you| F[Syncthing folders there]
  C --> E[Syncthing folders here]
  E <-->|your files, over Tailscale| F
```

- Your choices and each computer's status (which revision it uses, whether it waits for you, its
  Tailscale address, folder fingerprints) are rows in your Brahma account, end-to-end encrypted.
  The server only ever sees ciphertext.
- File lists and per-file fingerprints, which are too big for rows, travel between your
  computers in Syncthing's `setu-control` folder, which Setu creates and shares for you.
- Each folder whose direction differs from the folder around it becomes its own Syncthing
  folder: *send only* on the sending computer, *receive only* on the others, or *send and
  receive* for Both ways.
- Files and folders set to *Don't sync* are added to that Syncthing folder's ignore list. A file
  can have its own direction only when the folder it sits in is *Don't sync*.
- Setu only touches Syncthing folders whose ID starts with `setu-f-`. If another Syncthing folder
  overlaps a folder Setu manages, Setu stops and tells you instead of guessing.

## ❓ FAQ

<details>
<summary><b>Does Setu send my files anywhere?</b></summary>

No. Files go directly between your computers through Syncthing, over Tailscale. Your Brahma
server only holds your sync choices and each computer's status, encrypted so the server cannot
read them.
</details>

<details>
<summary><b>How do I copy everything from Windows to Linux, but leave out some big projects?</b></summary>

On the Linux computer:

1. Press **Quick setup** and choose **Copy everything from Windows to this computer**.
2. Press **Next**. Untick the projects to leave out. The biggest folders are at the top, and
   anything over 1 GB is marked *large*.
3. Press **Review**, check the numbers and press **Start syncing**.
4. Windows picks this up by itself. Its Setu only asks you if the change would replace many
   files there.

To leave out something deeper, such as one project inside a folder, click it in the list and
choose **Don't sync**. To leave out every file of a kind wherever it is, such as all `*.iso`
disk images, click **Everything**, open **Always left out** and add the pattern. Build output (`node_modules`, `target`, `build`, `dist`, `.git`) is
always left out, so code projects are usually much smaller than they look on disk.

What this means day to day: edits and deletes on Windows reach Linux. Edits you make on Linux
stay on Linux and are not sent back. Files that exist only on Linux are kept. A project you
left out is not touched on either computer.
</details>

<details>
<summary><b>What happens if I edit a file on the receiving computer?</b></summary>

The change stays on that computer and is not sent back. Syncthing marks the folder as having
local changes. The sending computer's version wins again if you revert those changes in
Syncthing.
</details>

<details>
<summary><b>What does Setu delete on its own?</b></summary>

Only sync leftovers that are exact copies: a `sync-conflict` copy, or an old version in
`.stversions`, that has the same size, the same SHA-256 and the same bytes as the file it belongs
to. Two files of your own that happen to be the same are never touched. Each removal is written
to a log, listed under **Same everywhere? → Duplicates**, and can be put back.
</details>

<details>
<summary><b>Where do replaced and deleted files go?</b></summary>

Into a hidden `.stversions` folder inside each synced folder, kept for 30 days. Open
**Old versions** in Setu and press **Put back**. A conflict copy you do not keep goes there too.
</details>

<details>
<summary><b>Can I use Setu with three or more computers?</b></summary>

Yes, up to eight. Sign in on each one with the same Brahma account. A direction such as *Copy
from Windows* means that computer sends and all the others receive.
</details>

## 🛠 For developers

Setu is Rust and Svelte 5 in a [Tauri 2](https://tauri.app) window, plus a command-line tool
that does everything the window does.

| Folder | What is in it |
| --- | --- |
| `core/` | Rules, the compiler that turns rules into Syncthing folders and ignore lists, planning and applying, the Brahma link (`hub.rs`), pairing (`mesh.rs`), content checks (`accuracy.rs`) and duplicate removal (`tidy.rs`) |
| `cli/` | The `setu` command and `setu-watch`, the background helper |
| `app/` | The window: Svelte front end in `src/`, Tauri back end in `src-tauri/` |
| `packaging/` | The Linux download (`linux/package.sh`) and the Windows installer (`windows/setu.iss`, Inno Setup) |

```sh
cargo test -p setu-core                         # fast tests
cargo test -p setu-core -- --ignored            # real Syncthings and a Brahma server on this computer
cargo run -p setu-cli -- status                 # the command-line tool
cd app && npm ci && npx tauri build --no-bundle # the window
```

The slow tests start their own Syncthing instances with temporary folders and ports, and a
throwaway Brahma server on loopback (Node 24+, from a `brahma-life-suite` checkout next to this
one, or set `BRAHMA_SERVER_TS` to its `server/src/home.ts`). They never touch the Syncthing you
use. The Brahma sync client is the `brahma-sync` crate from brahma core (`sync-rs/`).

A tag such as `v0.1.0` runs `.github/workflows/release.yml`, which builds both downloads and
publishes a GitHub release.

## Licence

MIT. See [LICENSE](LICENSE).
