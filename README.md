# Wisdom

Windows Minecraft launcher written in Rust and Tauri. Program files are installed below
`%LOCALAPPDATA%\Programs\Zorblock\Wisdom`; launcher data, Minecraft versions and instances live
separately below `%LOCALAPPDATA%\Zorblock\Wisdom`. Existing data from the former Roaming path is
migrated automatically on first start.

## Development

```powershell
npm run dev       # start the launcher
npm run check     # compile check
npm run build     # build the Tauri frontend
npm run build:release # build Wisdom.exe without an installer
npm run release   # create the Windows x64 portable app and installer
```

`npm run release` currently targets Windows x64 only. It writes the distributable files and
SHA-256 checksums to `release/<version>/windows-x64/` and the manifest to
`release/<version>/release.json`. The command does not upload or publish anything.

## Microsoft login

Microsoft sign-in uses browser OAuth with PKCE and obtains Xbox Live, XSTS and Minecraft
access tokens. Wisdom's public Microsoft client ID is part of the program, so users do not need
to edit a configuration file. Never add a client secret: desktop public clients must not have one.
The refresh token and Minecraft session are protected in the Windows Credential Manager; they are
never written as clear text into the launcher folder.

## Current foundation

- Clean desktop library with editable, isolated instances and per-instance launch settings.
- Mojang's official version manifest is cached locally; release/snapshot versions can be selected.
- Starting a version downloads its version metadata, client, libraries, native Windows libraries
  game assets and logging configuration to the launcher user-data folder, then invokes Java with
  the generated classpath.
- A valid Microsoft Minecraft account is required to start the game. The required Temurin Java
  runtime is downloaded automatically into the Wisdom user-data folder.
- Long-running authentication, setup and download work runs outside the UI thread and reports
  status/progress in the launcher.
