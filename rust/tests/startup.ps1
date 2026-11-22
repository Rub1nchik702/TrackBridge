param(
    [Parameter(Mandatory)][string] $Executable
)
$ErrorActionPreference = 'Stop'
$source = (Resolve-Path -LiteralPath $Executable).Path
if ([Diagnostics.FileVersionInfo]::GetVersionInfo($source).FileDescription -cne 'TrackBridge') {
    throw 'Task Manager process description must be TrackBridge.'
}
$run = Join-Path ([IO.Path]::GetTempPath()) ('TrackBridge-startup-' + [guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $run
$exe = Join-Path $run 'TrackBridge.exe'
Copy-Item -LiteralPath $source -Destination $exe
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class TrackBridgeWindowProbe {
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hwnd, int command);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageTimeoutW(IntPtr hwnd, uint msg, UIntPtr wp, IntPtr lp, uint flags, uint timeout, out UIntPtr result);
}
'@
foreach ($mode in 'normal', 'overview', 'settings') {
    $options = @{FilePath=$exe; WorkingDirectory=$run; WindowStyle='Hidden'; PassThru=$true}
    if ($mode -eq 'overview') { $options.ArgumentList = @('--preview-ui') }
    if ($mode -eq 'settings') { $options.ArgumentList = @('--preview-ui','--preview-settings') }
    $proc = Start-Process @options
    try {
        $timer = [Diagnostics.Stopwatch]::StartNew()
        do {
            Start-Sleep -Milliseconds 100
            $proc.Refresh()
        } until ($proc.HasExited -or $proc.MainWindowTitle -like '*Twitch*' -or $timer.Elapsed.TotalSeconds -gt 12)
        if (!$proc.HasExited -and $proc.MainWindowHandle -ne [IntPtr]::Zero) {
            $null = [TrackBridgeWindowProbe]::ShowWindowAsync($proc.MainWindowHandle,4)
        }
        if ($mode -eq 'normal') {
            if ($proc.HasExited -or $proc.MainWindowTitle -notlike '*Twitch*') {throw "Main window missing. Exited=$($proc.HasExited), code=$($proc.ExitCode), title=$($proc.MainWindowTitle). $run"}
            for ($i=0; $i -lt 12; $i++) {
                $reply=[UIntPtr]::Zero
                $ok=[TrackBridgeWindowProbe]::SendMessageTimeoutW($proc.MainWindowHandle,0,[UIntPtr]::Zero,[IntPtr]::Zero,2,1000,[ref]$reply)
                if ($ok -eq [IntPtr]::Zero) {throw "UI stopped processing messages. $run"}
                Start-Sleep -Milliseconds 250
            }
            if (!$proc.CloseMainWindow()) {throw "Could not close main window. $run"}
        }
        if (!$proc.WaitForExit(12000)) {throw "$mode timed out. $run"}
        if ($proc.ExitCode -ne 0) {throw "$mode exited with code $($proc.ExitCode). $run"}
        if ($mode -ne 'normal') {
            $name = if ($mode -eq 'settings') {'ui-settings-preview.png'} else {'ui-preview.png'}
            if ((Get-Item -LiteralPath (Join-Path $run $name)).Length -lt 1000) {throw "Missing preview. $run"}
        }
        Write-Output "PASS $mode"
    } finally {
        if (!$proc.HasExited) {$proc.Kill(); $proc.WaitForExit()}
        $proc.Dispose()
    }
}
Write-Output "Isolated test directory: $run"
