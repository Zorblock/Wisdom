# Wisdom

Windows Minecraft launcher written in Rust and Slint. It stores every launcher file below
`C:\Users\Jonas\AppData\Roaming\zorblock\userData\Wisdom` (using the current Windows user's
Roaming profile at runtime).

## Development

```powershell
npm run dev       # start the launcher
npm run check     # compile check
npm run build     # optimized exe in target\release
npm run setup     # build and create installer\output\Wisdom-Setup.exe
```

## Microsoft login

Microsoft sign-in uses the OAuth device-code flow and obtains Xbox Live, XSTS and Minecraft
access tokens. Create an Azure app registration configured for public-client/device-code login,
then enter its **Application (client) ID** in:

`C:\Users\Jonas\AppData\Roaming\zorblock\userData\Wisdom\config.json`

The launcher creates this file on first start. Never place a client secret in it: desktop public
clients must not have one. The refresh token and Minecraft session are protected in the Windows
Credential Manager; they are never written as clear text into the launcher folder.

## Current foundation

- Mojang's official version manifest is loaded and release/snapshot versions can be selected.
- Starting a version downloads its version metadata, client, libraries, native Windows libraries
  and game assets to the launcher user-data folder, then invokes Java with the generated classpath.
- A valid Microsoft Minecraft account is required to start the game. Java 21+ should be installed
  and available as `java` (or configured via `JAVA_HOME`).
