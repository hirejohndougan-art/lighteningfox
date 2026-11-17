# Opens Ironswift's real window on Windows (GitHub's windows-latest machine has a desktop),
# types and clicks in it, and prints small desktop screenshots into the log as base64 between
# BEGIN-SHOT/END-SHOT lines, for reviewers who can't download artifacts. Fails if the browser
# crashes or never shows its first frame.
#   powershell -File tools/windows_smoke.ps1
$ErrorActionPreference = "Continue"
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Mouse {
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, IntPtr e);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  public static void Click(int x, int y) { SetCursorPos(x, y); mouse_event(2, 0, 0, 0, IntPtr.Zero); mouse_event(4, 0, 0, 0, IntPtr.Zero); }
}
"@

New-Item -ItemType Directory -Force shots | Out-Null
function Shot($name) {
  $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
  $full = New-Object System.Drawing.Bitmap $b.Width, $b.Height
  $g = [System.Drawing.Graphics]::FromImage($full)
  $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
  $w = [int]($b.Width / 2); $h = [int]($b.Height / 2)
  $small = New-Object System.Drawing.Bitmap $w, $h
  $g2 = [System.Drawing.Graphics]::FromImage($small)
  $g2.InterpolationMode = "HighQualityBicubic"
  $g2.DrawImage($full, 0, 0, $w, $h)
  $path = "shots\win-$name.png"
  $small.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
  Write-Output "BEGIN-SHOT $name"
  Write-Output ([Convert]::ToBase64String([IO.File]::ReadAllBytes($path)))
  Write-Output "END-SHOT"
}

$exe = Resolve-Path "target\release\ironswift.exe"
$env:IRONSWIFT_PROFILE = Join-Path $PWD "win-profile"
$env:IRONSWIFT_NO_UPDATES = "1"

# 1. Time to the first frame on screen.
$env:IRONSWIFT_EXIT_AFTER_FIRST_FRAME = "1"
$first = & $exe tests\forms.html 2>&1 | Out-String
Remove-Item Env:IRONSWIFT_EXIT_AFTER_FIRST_FRAME
Write-Output "first-frame run: $first"

# 2. The window itself.
$page = Resolve-Path "tests\forms.html"
$p = Start-Process -FilePath $exe -ArgumentList "`"$page`"" -PassThru -RedirectStandardError "win-stderr.txt" -RedirectStandardOutput "win-stdout.txt"
Start-Sleep -Seconds 12
$shell = New-Object -ComObject WScript.Shell
$null = $shell.AppActivate($p.Id)
Start-Sleep -Seconds 1
Shot "1-loaded"

# 3. Click the first text field ("Your name") and type into it.
$r = New-Object Mouse+RECT
$null = [Mouse]::GetWindowRect([Mouse]::GetForegroundWindow(), [ref]$r)
Write-Output "window rect: $($r.Left),$($r.Top) - $($r.Right),$($r.Bottom)"
[System.Windows.Forms.SendKeys]::SendWait("{TAB}")
Start-Sleep -Milliseconds 500
[System.Windows.Forms.SendKeys]::SendWait("Hello from Windows")
Start-Sleep -Seconds 1
Shot "2-typed"

# 4. A new tab with typing in the address bar, then a click in the page.
[System.Windows.Forms.SendKeys]::SendWait("^t")
Start-Sleep -Seconds 1
[System.Windows.Forms.SendKeys]::SendWait("ironswift on windows")
Start-Sleep -Seconds 1
Shot "3-new-tab"
[System.Windows.Forms.SendKeys]::SendWait("{ESC}")
[System.Windows.Forms.SendKeys]::SendWait("^w")
Start-Sleep -Seconds 1
[Mouse]::Click([int](($r.Left + $r.Right) / 2), [int]($r.Top + ($r.Bottom - $r.Top) * 0.6))
Start-Sleep -Seconds 1
Shot "4-after-click"

$alive = -not $p.HasExited
if ($alive) { Stop-Process -Id $p.Id -Force }
Start-Sleep -Seconds 1
Write-Output "----- browser output -----"
Get-Content "win-stdout.txt", "win-stderr.txt" -ErrorAction SilentlyContinue | Select-Object -First 80
if (-not $alive) { Write-Output "BROWSER EXITED EARLY (exit code $($p.ExitCode))"; exit 1 }
if ($first -notmatch "first frame on screen after") { Write-Output "NO FIRST FRAME"; exit 1 }
Write-Output "window test passed"
