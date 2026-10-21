# End-to-end check: opens Notepad, presses the Murmur hotkey (hold), and prints what was pasted.
# Needs a running Murmur that was started with MURMUR_FAKE_WAV set (replaces the microphone).
param([int]$HoldMs = 1500, [string]$Shot = "")
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public class K { [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, string title);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, System.Text.StringBuilder l); }
"@
$np = Start-Process (Join-Path $env:WINDIR "notepad.exe") -PassThru
Start-Sleep -Seconds 2
[K]::SetForegroundWindow($np.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 500
$CTRL = 0x11; $SHIFT = 0x10; $SPACE = 0x20
[K]::keybd_event($CTRL, 0, 0, [UIntPtr]::Zero); [K]::keybd_event($SHIFT, 0, 0, [UIntPtr]::Zero); [K]::keybd_event($SPACE, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds ($HoldMs / 2)
if ($Shot) { [System.Drawing.Bitmap]$b = New-Object System.Drawing.Bitmap ([System.Windows.Forms.Screen]::PrimaryScreen.Bounds.Width), ([System.Windows.Forms.Screen]::PrimaryScreen.Bounds.Height); $g = [System.Drawing.Graphics]::FromImage($b); $g.CopyFromScreen(0, 0, 0, 0, $b.Size); $b.Save($Shot) }
Start-Sleep -Milliseconds ($HoldMs / 2)
[K]::keybd_event($SPACE, 0, 2, [UIntPtr]::Zero); [K]::keybd_event($SHIFT, 0, 2, [UIntPtr]::Zero); [K]::keybd_event($CTRL, 0, 2, [UIntPtr]::Zero)
Start-Sleep -Seconds 5
$edit = [K]::FindWindowEx($np.MainWindowHandle, [IntPtr]::Zero, "Edit", $null)
$buffer = New-Object System.Text.StringBuilder 4096
[K]::SendMessage($edit, 0xD, [IntPtr]4096, $buffer) | Out-Null
"PASTED: [" + $buffer.ToString() + "]"
Stop-Process -Id $np.Id -Force
