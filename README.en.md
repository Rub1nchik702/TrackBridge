# TrackBridge

[Русский](README.md) · **English**

<img src="assets/TrackBridge-icon.png" width="96" alt="TrackBridge icon">

**By Rub1nchik · Windows · 1.8.6 beta**

TrackBridge connects a custom Twitch Channel Points reward to your Spotify queue. Viewers submit a song link, the app finds a matching Spotify track and adds it to the end of your queue without interrupting the current song.

## Screenshots

Overview and settings in version 1.8.6, shown with demonstration data.

![TrackBridge overview](assets/interface-overview.png)

![TrackBridge settings](assets/interface-settings.png)

## Features

- Direct Spotify track links.
- Spotify matching for YouTube and YouTube Music links. Text in round or square brackets in YouTube titles is ignored.
- Automatic point refunds and Twitch chat notifications when no confident match is found.
- Twitch EventSub with fallback checks for pending redemptions every 10 seconds.
- Duplicate prevention and manual resolution of uncertain playback requests.
- System tray support and Windows DPAPI protection for stored tokens.
- Automatic GitHub release checks, verified downloads and restart-to-install updates.
- Russian and English UI. Use the RU/EN buttons in the sidebar; changes apply immediately and are saved. Account IDs and custom reward titles are preserved. Earlier log entries stay in their original language. New chat notifications use the selected language.

## Download

Get the Windows ZIP from [Releases](https://github.com/Rub1nchik702/TrackBridge/releases). Extract the entire archive to a writable folder and run `TrackBridge.exe`.

Requires Windows 10/11, .NET Framework 4.7.2 or later, Spotify Premium and an active Spotify player. Your Twitch channel must support custom Channel Points rewards. The executable is currently unsigned. Source code is closed; obfuscation makes inspection harder but cannot prevent reverse engineering.

## Setup

1. Create your own Web API app in the [Spotify Developer Dashboard](https://developer.spotify.com/dashboard). Set the exact redirect URI to `http://127.0.0.1:43821/callback`.
2. Paste its **Client ID** into TrackBridge, click **Connect Spotify** and authorize in your browser. No Client Secret is needed. If required, add your Spotify account to the developer app's allowed users.
3. Register a **Public** app in the [Twitch Developer Console](https://dev.twitch.tv/console/apps). If a redirect URL is required, enter `http://localhost:43821`. Device Code Flow does not use this redirect.
4. Paste the Twitch Client ID and click **Connect Twitch**. Authorize as the channel owner and allow chat messages for refund notifications.
5. Create a reward through TrackBridge, or select one previously created using the same Twitch Client ID. Require text input and keep redemptions in the request queue.
6. Open Spotify and start playback on your preferred device. Select the reward in TrackBridge and click **Start**.
7. Wait for **Requests active · Twitch connected**, then submit a test request using that reward.

Connecting accounts does not start request handling. Each broadcaster currently supplies their own developer application IDs.

## Automatic updates

Starting with version 1.8, TrackBridge checks the public `Rub1nchik702/TrackBridge` GitHub Releases feed at startup and every six hours, including beta releases. It downloads a newer Windows archive into `.updates` next to the app and verifies the SHA-256 digest supplied by GitHub.

When ready, click **Install update** and confirm the restart. Request handling stops cleanly; only packaged application files are replaced. Your `data` folder is preserved. If replacement fails, files already changed are rolled back. You can also click **Check for updates** manually. GitHub outages do not stop song requests.

Version 1.7 needs a one-time manual upgrade to 1.8 or later. Close the old app, replace its files and keep `data`. The updater requires write access to the installation folder.

## Beta limitations

- Matching may miss a song that exists in Spotify. Direct Spotify links are more reliable.
- Songs are appended to the existing queue. Reward fulfillment confirms that Spotify accepted the queue request, not that the song played to completion.
- If a Spotify response is lost, the request is marked **Check manually** instead of being resent. Inspect the Spotify queue and use the request's context menu to confirm or refund it.
- Starting the app also recovers pending redemptions for the selected reward, including purchases made while the app was closed. Cancel unwanted redemptions in Twitch before starting.
- OBS now-playing output and a clickable Twitch Extension are not included in this release.

## Privacy and support

Client IDs, tokens and request history are stored in `data/settings.bin`, protected using Windows DPAPI for the current Windows account. Do not upload or share `data`. DPAPI does not protect against software running as the same Windows user. To reset, close TrackBridge and rename `data`. Account access can be revoked in Twitch and Spotify settings.

When reporting a problem, include the version, steps and sanitized log lines. Do not share settings files, tokens, authorization codes, Client Secrets or viewers' private information.

## License

By **Rub1nchik**. See [LICENSE.txt](LICENSE.txt) and [THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt). This repository contains public documentation and release materials only.

TrackBridge controls your own Spotify player and does not grant broadcasting rights. Premium does not authorize streaming music to an audience. Follow [Spotify's policies](https://developer.spotify.com/policy) and Twitch's rules. TrackBridge is not affiliated with Spotify, Twitch, YouTube. Paid access and donation features are not enabled.

## Queue and updater protections in 1.8.2

The app retains up to 100 active requests, 500 completed history entries and 5000 recent receipt IDs. Each viewer can have up to three active requests. When the global queue is full, additional redemptions remain pending in Twitch until space becomes available. Temporary failures are deferred rather than blocking the queue; after three failed attempts a refund is requested. Failed Twitch finalization requires manual resolution.

Future updates require a publisher-signed RSA/SHA-256 manifest verified against a pinned public key. For a trusted transition, install 1.8.2 manually from a trusted package: 1.8 and 1.8.1 cannot verify publisher signatures. This package signature is separate from Windows Authenticode code signing.

## Chat commands

- `!song` or `!nowplaying` replies with the current Spotify track and link, with a shared 5-second reply cooldown.
- `!skip` or `!voteskip` votes to skip. Automatic mode requires 50% of viewers, rounded up (11 viewers → 6 votes). Below 100 viewers, the owner can choose a manual threshold of 1–99 votes. At 100 or more viewers, 50% always applies.
- Twitch viewer count is fetched at the first vote and the threshold is fixed for that track. No new votes are accepted without a live viewer count. Duplicate Twitch user IDs are ignored. Votes reset on track changes or rewinding to the beginning. Change settings while stopped. Skipping does not refund channel points.
- Click Start and keep Spotify active. Reconnect Twitch once to grant permission to read and send chat messages.

## Controls and moderation

The live panel shows the current track, Twitch viewer count and vote progress. Click the track to open Spotify. Viewer count refreshes about every 30 seconds; the vote threshold remains fixed for the track.

Use Skip track or Reset votes while requests are running. Voting enabled can disable viewer votes while keeping song commands. Resetting votes does not unlock a skip with an uncertain Spotify response.

While stopped, open Request moderation to set a duration limit (10 minutes by default; 0 disables it), block Explicit tracks, and edit blocked track/artist lists. Use one entry per line. Matching is exact and case insensitive. Tracks accept Spotify links or exact titles; title bans apply to all artists. Artist entries use the exact Spotify name. Explicit relies on Spotify metadata and cannot guarantee filtering every unwanted lyric.

Rules apply before queueing pending and new requests, including direct Spotify links. Tracks already queued are not removed. Rejected orders refund points; unavailable metadata is handled as a temporary failure.

## Interface

Use Overview, Queue, Moderation and Settings in the sidebar. Settings contains account connections, the Twitch reward, voting threshold and updates. Start and stop requests from Overview.

The player card shows the track title, artist, Spotify cover when available and playback progress. The voting card displays votes and the fixed threshold. The bridge and waveform logo is also used for the application and tray icons.
