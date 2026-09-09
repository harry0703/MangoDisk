// Task recipes contain only their native operations. Step execution, redaction, progress,
// error classification and verification diagnostics are shared by every recipe.
pub(super) fn recipe(task_id: &str) -> Option<&'static str> {
    Some(match task_id {
        "windows.maintenance.update-components" => {
            r#"
foreach ($name in @('bits','cryptsvc','wuauserv')) { Invoke-MangoService $name }
Invoke-MangoStep 'usoClient' 'RequestScan' {
    $uso = Join-Path $windowsDirectory 'System32\UsoClient.exe'
    Start-Process -FilePath $uso -ArgumentList StartScan -WindowStyle Hidden -ErrorAction Stop
}
"#
        }
        "windows.maintenance.search-index" => {
            r#"
Invoke-MangoService 'WSearch' 'Stop'
try {
    Invoke-MangoStep 'SearchSetting' 'Write' {
        Set-ItemProperty -Path 'HKLM:\SOFTWARE\Microsoft\Windows Search' -Name SetupCompletedSuccessfully -Type DWord -Value 0 -ErrorAction Stop
    }
} finally { Invoke-MangoService 'WSearch' 'Start' }
"#
        }
        "windows.maintenance.print-queue" => {
            r#"
Invoke-MangoService 'Spooler' 'Stop'
try {
    Invoke-MangoStep 'PrintQueue' 'Delete' {
        $queue = Join-Path $windowsDirectory 'System32\spool\PRINTERS'
        if (Test-Path -LiteralPath $queue -ErrorAction Stop) {
            Get-ChildItem -LiteralPath $queue -Force -ErrorAction Stop | Remove-Item -Force -ErrorAction Stop
        }
    }
} finally { Invoke-MangoService 'Spooler' 'Start' }
"#
        }
        "windows.maintenance.audio-service" => "Invoke-MangoService 'Audiosrv' 'Restart'",
        "windows.maintenance.system-integrity" => {
            r#"
Invoke-MangoNative 'Dism' "$windowsDirectory\System32\dism.exe" @('/Online','/Cleanup-Image','/RestoreHealth') 'Execute' 'repairingComponentImage'
Invoke-MangoNative 'Sfc' "$windowsDirectory\System32\sfc.exe" @('/scannow') 'Execute' 'checkingSystemFiles'
"#
        }
        "windows.maintenance.performance-counters" => {
            r#"
Invoke-MangoNative 'Lodctr' "$windowsDirectory\System32\lodctr.exe" @('/R')
$wow = Join-Path $windowsDirectory 'SysWOW64\lodctr.exe'
if (Test-Path -LiteralPath $wow) { Invoke-MangoNative 'LodctrWow' $wow @('/R') }
Invoke-MangoNative 'Winmgmt' "$windowsDirectory\System32\wbem\winmgmt.exe" @('/resyncperf')
# lodctr /q can return 259 after printing a valid enabled provider. Read structured
# data from the same OS provider instead of treating that enumeration exit as failure.
Invoke-MangoStep 'PerformanceData' 'Query' {
    $sample = Get-CimInstance -ClassName Win32_PerfRawData_PerfOS_Processor -OperationTimeoutSec 30 -ErrorAction Stop | Select-Object -First 1
    if ($null -eq $sample -or $null -eq $sample.PercentProcessorTime -or $null -eq $sample.Timestamp_PerfTime) {
        throw [System.IO.InvalidDataException]::new('Performance data is unavailable')
    }
} 'Verify'
"#
        }
        "windows.maintenance.time-sync" => {
            r#"
Invoke-MangoService 'W32Time' 'Start'
Invoke-MangoNative 'W32Time' "$windowsDirectory\System32\w32tm.exe" @('/resync','/force')
Invoke-MangoNative 'W32Time' "$windowsDirectory\System32\w32tm.exe" @('/query','/status') 'Verify'
"#
        }
        "windows.maintenance.system-disk" => {
            r#"Invoke-MangoNative 'DiskCheck' "$windowsDirectory\System32\chkdsk.exe" @($windowsDirectory.Substring(0,2),'/scan')"#
        }
        "windows.maintenance.dns-cache" => {
            r#"Invoke-MangoNative 'Ipconfig' "$windowsDirectory\System32\ipconfig.exe" @('/flushdns')"#
        }
        "windows.maintenance.store-cache" => {
            r#"Invoke-MangoNative 'StoreCache' "$windowsDirectory\System32\wsreset.exe" @()"#
        }
        _ => return None,
    })
}
