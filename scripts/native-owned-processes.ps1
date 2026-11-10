param(
    [Parameter(Mandatory = $true)][int]$OwnedProcessId,
    [Parameter(Mandatory = $true)][string]$ExpectedExecutable
)
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
    throw 'Owned native diagnostics require a disposable hosted CI runner'
}
$taskProcesses = @(Get-CimInstance Win32_Process)
$taskRoot = $taskProcesses | Where-Object ProcessId -eq $OwnedProcessId | Select-Object -First 1
if (-not $taskRoot -or $taskRoot.ExecutablePath -ne $ExpectedExecutable) {
    throw 'Owned process exited or its executable identity changed'
}
$taskProcessIds = @($OwnedProcessId)
for ($level = 0; $level -lt 6; $level++) {
    $taskProcessIds += @($taskProcesses | Where-Object { $_.ParentProcessId -in $taskProcessIds } | Select-Object -ExpandProperty ProcessId)
}
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
@{
    host_elevated = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    processes = @($taskProcesses | Where-Object { $_.ProcessId -in $taskProcessIds } | ForEach-Object {
        @{
            name = $_.Name
            pid = $_.ProcessId
            parent = $_.ParentProcessId
            command = $_.CommandLine
            version = if ($_.ExecutablePath) { (Get-Item -LiteralPath $_.ExecutablePath).VersionInfo.FileVersion } else { $null }
        }
    })
} | ConvertTo-Json -Depth 5
