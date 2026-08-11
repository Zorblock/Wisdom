# Wisdom

Windows Minecraft launcher written in Rust and Tauri. It stores every launcher file below
`C:\Users\Jonas\AppData\Roaming\zorblock\userData\Wisdom` (using the current Windows user's
Roaming profile at runtime).

## Development

```powershell
npm run dev       # start the launcher
npm run check     # compile check
npm run build     # build the Tauri frontend
npm run build:release # build Wisdom.exe without an installer
npm run setup     # build and create the Inno Setup installer
```

## Microsoft login

Microsoft sign-in uses browser OAuth with PKCE and obtains Xbox Live, XSTS and Minecraft
access tokens. Wisdom's public Microsoft client ID is part of the program, so users do not need
to edit a configuration file. Never add a client secret: desktop public clients must not have one.
The refresh token and Minecraft session are protected in the Windows Credential Manager; they are
never written as clear text into the launcher folder.

## Current foundation

- Mojang's official version manifest is loaded and release/snapshot versions can be selected.
- Starting a version downloads its version metadata, client, libraries, native Windows libraries
  and game assets to the launcher user-data folder, then invokes Java with the generated classpath.
- A valid Microsoft Minecraft account is required to start the game. The required Temurin Java
  runtime is downloaded automatically into the Wisdom user-data folder.
