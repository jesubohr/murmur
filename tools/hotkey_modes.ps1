# Tap-to-toggle and Esc-to-cancel check. Read the result from the Murmur log.
Add-Type @"
using System; using System.Runtime.InteropServices;
public class K2 { [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra); }
"@
function Chord($ms) {
  [K2]::keybd_event(0x11, 0, 0, [UIntPtr]::Zero); [K2]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero); [K2]::keybd_event(0x20, 0, 0, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds $ms
  [K2]::keybd_event(0x20, 0, 2, [UIntPtr]::Zero); [K2]::keybd_event(0x10, 0, 2, [UIntPtr]::Zero); [K2]::keybd_event(0x11, 0, 2, [UIntPtr]::Zero)
}
Chord 120; Start-Sleep -Seconds 2; Chord 120; Start-Sleep -Seconds 5      # tap, record, tap again
Chord 120; Start-Sleep -Seconds 1
[K2]::keybd_event(0x1B, 0, 0, [UIntPtr]::Zero); [K2]::keybd_event(0x1B, 0, 2, [UIntPtr]::Zero); Start-Sleep -Seconds 2   # tap, then Esc
