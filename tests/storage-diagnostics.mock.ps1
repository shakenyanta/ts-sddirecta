param([string]$Mode)
function Get-WinEvent {
    param($FilterHashtable, $MaxEvents, $ErrorAction)
    if ($FilterHashtable.StartTime.Kind -ne [DateTimeKind]::Local -or
        $FilterHashtable.StartTime.ToUniversalTime() -ne [DateTimeOffset]::Parse('2026-11-05T23:30:06.551Z').UtcDateTime) {
        throw 'Event window must preserve the instant and use local DateTime for Get-WinEvent.'
    }
    if ($Mode -eq 'no-events') { throw 'Simulated event log unavailable' }
    1..30 | ForEach-Object {
        [pscustomobject]@{
            ProviderName = 'disk'; Id = 153; RecordId = $_; TimeCreated = [datetime]'2026-11-06T08:31:36'
            Message = ('x' * 2000); Properties = @([pscustomobject]@{ Value = [byte[]]@(1, 2, 3) })
        }
    }
}
function Get-CimInstance {
    param($ClassName, $Filter, $OperationTimeoutSec, $ErrorAction, $Namespace)
    if ($Mode -eq 'device-error') { throw 'Simulated device query unavailable' }
    if ($ClassName -eq 'MSFT_Partition') {
        [pscustomobject]@{ DiskNumber = 77; PartitionNumber = 1; Offset = 0; Size = 512000000 }
    } elseif ($ClassName -eq 'Win32_DiskDrive') {
        [pscustomobject]@{ Index = 77; Size = 512000000; Model = 'Mock Reader'; PNPDeviceID = 'USBSTOR\MOCK'; Status = 'OK' }
    } elseif ($ClassName -eq 'Win32_PnPEntity') {
        [pscustomobject]@{ Name = 'Mock Reader'; Service = 'USBSTOR'; Status = 'OK'; ConfigManagerErrorCode = 0 }
    } elseif ($ClassName -eq 'Win32_PnPSignedDriver') {
        [pscustomobject]@{ DriverProviderName = 'Mock Provider'; DriverVersion = '1.2.3'; InfName = 'mock.inf' }
    } else { throw 'Unexpected class' }
}
function Get-CimAssociatedInstance {
    param($InputObject, $Association, $ResultClassName, $OperationTimeoutSec, $ErrorAction)
    if ($Mode -eq 'volume-error') { throw 'Simulated volume query unavailable' }
    [pscustomobject]@{
        DriveLetter = 'G'; FileSystem = $(if ($Mode -eq 'raw-volume') { 'RAW' } else { 'FAT32' })
        FileSystemType = $(if ($Mode -eq 'raw-volume') { 0 } else { 3 }); Size = 511000000
        HealthStatus = 0; OperationalStatus = @(2)
    }
}
function Get-PnpDeviceProperty {
    param($InstanceId, $KeyName, $ErrorAction)
    # A cyclic parent must terminate rather than generate unbounded output.
    [pscustomobject]@{ Data = 'USBSTOR\MOCK' }
}
. "$PSScriptRoot/../src-tauri/src/storage_diagnostics.ps1"
