JondisClip
==========

Always-on game clipping for Windows. It quietly keeps the last few minutes
of your screen and sound, and saves them when you press a hotkey. It's
built to stay out of your game's way.


Getting started
---------------
1. Unzip this folder somewhere it can stay, for example
   C:\Users\<you>\JondisClip. Keep all the files together.
2. Double-click jondisclip.exe.
   Windows may say "Windows protected your PC", because the app isn't
   signed. Click "More info", then "Run anyway". You only need to do
   this once.
3. The first time, a welcome window opens where you choose your hotkey,
   where clips are saved, and whether to record your microphone.
   JondisClip is now in the system tray (by the clock; check the ^ arrow)
   and starts recording straight away.
4. Press Alt+F10 to save the last 5 minutes. You'll hear a short sound and
   get a notification when the clip is saved.

Clips go to Videos\JondisClip, in a folder per game.


The library
-----------
Left-click the tray icon to open the library. There you can:
- play your clips, trim them, and save the trimmed part as a new clip
  (your originals are never changed)
- Share to Discord: makes a copy small enough for Discord and puts it on
  your clipboard. Paste it into any chat with Ctrl+V.
- Save to Drive (if you use Google Drive for desktop, pick its folder in
  Settings first)
- rename clips, make folders, and drag clips into them

Closing the library window doesn't stop recording. The tray keeps going.


Settings worth knowing
----------------------
Open Settings in the library window, or right-click the tray icon.
- Replay length, the hotkey and where clips are saved
- Microphone: OFF at first. Turn it on to record your voice too.
- Start with Windows: OFF at first.
- Quality, resolution and frame rate
- Theme colour
- Language (English, Norsk bokmål, Norsk nynorsk) and date format

Right-click the tray icon to pause recording, change the replay length,
or quit.


Graphics cards
--------------
JondisClip uses your graphics card's own video encoder, so it barely
affects your game. It's been tested on AMD Radeon. NVIDIA and Intel should
work but haven't been tested yet. If recording doesn't start, send the
log file (below) to whoever gave you JondisClip.


Where things are kept
---------------------
- Clips:    Videos\JondisClip (change it in Settings)
- Settings: %APPDATA%\JondisClip\settings.toml
- Log:      %LOCALAPPDATA%\JondisClip\jondisclip.log
- Buffer:   %LOCALAPPDATA%\JondisClip\buffer (temporary; cleared on quit)

To remove JondisClip: turn off "Start with Windows" in Settings, quit it
from the tray, then delete this folder. Delete the two JondisClip folders
above as well if you want, and Videos\JondisClip if you don't want your
clips.


Updates
-------
JondisClip asks GitHub now and then whether a newer version exists. When
there is one you get a notification. Right-click the tray icon and choose
"Install update" (or use the button in Settings). It downloads, installs
and restarts by itself; your clips and settings are kept. Nothing is
installed without your click, and you can turn the check off in Settings.
Settings also has a "Check now" button.


Privacy
-------
No account, no telemetry, nothing is uploaded. Clips only leave your PC
when you share them yourself, or when you use Save to Drive. The update
check only asks for the latest version number; it sends nothing about you.

Requires Windows 10 or 11, 64-bit. The library window uses Microsoft
WebView2, which is part of Windows 11 and comes with Microsoft Edge on
Windows 10.
