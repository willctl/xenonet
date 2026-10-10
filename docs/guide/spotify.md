# Spotify

XenoNet can be a remote for the Spotify app you already use. It shows what's playing under the status panel, with play or pause and the next track. While Spotify plays, XenoNet's own music steps aside, and it comes back when Spotify stops. If you turn it on, your volume dips while a session waits on you, and comes back after.

XenoNet plays no Spotify audio itself and never mixes it with its own music. Spotify's player needs Premium.

## Connecting

Spotify lets each app connect only a few accounts, so XenoNet connects through an app of your own:

1. On [Spotify's developer dashboard](https://developer.spotify.com/dashboard), create an app and tick the Web API.
2. Add `http://127.0.0.1/api/spotify/callback` as its redirect URI. Spotify adds XenoNet's port when you sign in.
3. In XenoNet, choose **Connect Spotify, or change what it does** in the palette, paste the app's client id, and choose **Connect**.
4. Sign in to Spotify in your browser. It sends you back to XenoNet, which says you can close the tab.

XenoNet keeps the sign-in's refresh token in Windows Credential Manager, the macOS keychain or the Secret Service on Linux. The rest stays in XenoNet's service and never reaches the page. **Disconnect**, in the same window, forgets it.

## Dipping the volume

In the same window, play something and choose **plays here** for this computer's Spotify, then turn on **Dip the volume while a session waits on you**. While any session waits for your answer, the volume on that device goes down to about a third, and goes back when nothing waits. If you change the volume meanwhile, yours stands. XenoNet never turns down any other device, such as a speaker in another room.

## What reaches Spotify

XenoNet talks to Spotify to sign in and refresh its token, to read what's playing every few seconds while a window is open (every 30 seconds otherwise), and to send the commands you give or turn on. Nothing of your sessions reaches Spotify, though the timing of a dip shows when something waited on you. What's playing never goes to SI (super intelligence), an actor or another machine.
