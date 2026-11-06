# Read-only failure evidence. No disk handles, format, reset, or repair commands.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::InputEncoding = New-Object System.Text.UTF8Encoding($false)
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$request = [Console]::In.ReadToEnd() | ConvertFrom-Json
function Emit($kind, $data) {
    [Console]::WriteLine((@{ kind = $kind; data = $data } | ConvertTo-Json -Compress -Depth 8))
}
function Short($value, $limit = 1200) {
    $s = [string]$value
    if ($s.Length -gt $limit) { return $s.Substring(0, $limit) + ' [truncated]' }
    return $s
}
$failure = [DateTimeOffset]::FromUnixTimeMilliseconds([long]$request.failure_ms).UtcDateTime
$start = $failure.AddSeconds(-90)
$end = $failure.AddSeconds(3)
Emit 'window' @{ start_utc = $start.ToString('o'); end_utc = $end.ToString('o'); correlation = 'time_window_only_not_proof_of_cause' }
if ($request.executable) {
    try {
        Emit 'application' @{ version = $request.version; sha256 = (Get-FileHash -LiteralPath $request.executable -Algorithm SHA256 -ErrorAction Stop).Hash }
    } catch { Emit 'application_query_error' @{ error = (Short $_.Exception.Message) } }
}
# Prioritize the volume state before potentially slow driver inventory queries.
# This is an observation after handles closed, not an assertion of its state at failure.
try {
    $parts = @(Get-CimInstance -Namespace 'root/Microsoft/Windows/Storage' -ClassName MSFT_Partition -Filter ('DiskNumber = {0}' -f [uint32]$request.disk) -OperationTimeoutSec 2 -ErrorAction Stop | Select-Object -First 4)
    foreach ($part in $parts) {
        try {
            $volumes = @(Get-CimAssociatedInstance -InputObject $part -Association MSFT_PartitionToVolume -ResultClassName MSFT_Volume -OperationTimeoutSec 2 -ErrorAction Stop | Select-Object -First 4)
            Emit 'volume_observed_after_failure' @{
                observed_utc = [DateTime]::UtcNow.ToString('o'); disk = $part.DiskNumber
                partition = $part.PartitionNumber; offset = $part.Offset; size = $part.Size
                volumes = @($volumes | ForEach-Object { @{
                    drive_letter = [string]$_.DriveLetter; file_system = [string]$_.FileSystem
                    file_system_type = $_.FileSystemType; size = $_.Size
                    health = $_.HealthStatus; operational_status = @($_.OperationalStatus)
                } })
                caveat = 'observed_after_failure; blank_or_unavailable_does_not_prove_RAW'
            }
        } catch { Emit 'volume_query_error' @{ disk = $request.disk; partition = $part.PartitionNumber; error = (Short $_.Exception.Message) } }
    }
    Emit 'partition_query' @{ disk = $request.disk; count = $parts.Count; output_limit = 4 }
} catch { Emit 'partition_query_error' @{ disk = $request.disk; error = (Short $_.Exception.Message) } }
foreach ($log in @('System', 'Microsoft-Windows-StorageVolume/Operational')) {
    try {
        # Windows PowerShell 5.1's event filter expects local DateTime values.
        # Keep UTC in the evidence, but explicitly convert the query boundaries.
        $filter = @{ LogName = $log; StartTime = $start.ToLocalTime(); EndTime = $end.ToLocalTime() }
        if ($log -eq 'System') { $filter.Level = @(1, 2, 3) }
        # MaxEvents also bounds unrelated events inspected in a busy System log.
        $events = @(Get-WinEvent -FilterHashtable $filter -MaxEvents 200 -ErrorAction Stop |
            Where-Object { $_.ProviderName -match '^(disk|Ntfs|volmgr|partmgr|stor.*|.*(StorageVolume|Kernel-PnP|USB|DriverFrameworks).*)$' } |
            Select-Object -First 24)
        foreach ($event in $events) {
            Emit 'windows_event' @{
                log = $log; provider = $event.ProviderName; id = $event.Id; record_id = $event.RecordId
                time_utc = $event.TimeCreated.ToUniversalTime().ToString('o'); message = (Short $event.Message)
                # Disk number / PDO / LBA and raw status fields, without guessing their meaning.
                properties = @($event.Properties | Select-Object -First 12 | ForEach-Object {
                    if ($_.Value -is [byte[]]) { Short ([BitConverter]::ToString($_.Value)) 1200 } else { Short $_.Value 400 }
                })
            }
        }
        Emit 'event_query' @{ log = $log; count = $events.Count; scan_limit = 200; output_limit = 24 }
    } catch {
        $none = $_.FullyQualifiedErrorId -like 'NoMatchingEventsFound*'
        Emit 'event_query' @{ log = $log; count = $(if ($none) { 0 } else { $null }); status = $(if ($none) { 'no_events' } else { 'unavailable' }); error = $(if ($none) { '' } else { Short $_.Exception.Message }) }
    }
}
try {
    $disk = Get-CimInstance -ClassName Win32_DiskDrive -Filter ('Index = {0}' -f [uint32]$request.disk) -OperationTimeoutSec 3 -ErrorAction Stop
    if (-not $disk) { throw 'The requested disk is no longer present.' }
    $matches = [uint64]$disk.Size -eq [uint64]$request.capacity
    Emit 'device_observed_after_failure' @{
        disk = $disk.Index; size = [uint64]$disk.Size; expected_capacity_matches = $matches
        model = $disk.Model; interface = $disk.InterfaceType; pnp_id = $disk.PNPDeviceID
        status = $disk.Status; config_error = $disk.ConfigManagerErrorCode
        identity_caveat = 'observed_after_failure; same_capacity_does_not_prove_same_card_or_reader'
    }
    # WMI size can differ from the exact native capacity. Keep this observed
    # chain as evidence, but never claim disk number/size proves device identity.
    $instance = [string]$disk.PNPDeviceID
    $seen = @{}
    for ($depth = 0; $depth -lt 4 -and $instance -and -not $seen.ContainsKey($instance); $depth++) {
        $seen[$instance] = $true
        $escaped = $instance.Replace('\', '\\').Replace("'", "\'")
        try {
            $device = Get-CimInstance -ClassName Win32_PnPEntity -Filter "DeviceID='$escaped'" -OperationTimeoutSec 3 -ErrorAction Stop
            $driver = Get-CimInstance -ClassName Win32_PnPSignedDriver -Filter "DeviceID='$escaped'" -OperationTimeoutSec 3 -ErrorAction Stop
            Emit 'driver_chain' @{
                depth = $depth; instance_id = $instance; name = $device.Name; service = $device.Service
                status = $device.Status; config_error = $device.ConfigManagerErrorCode
                provider = $driver.DriverProviderName; version = $driver.DriverVersion
                date = [string]$driver.DriverDate; inf = $driver.InfName; driver = $driver.DriverName
            }
        } catch { Emit 'driver_query_error' @{ depth = $depth; instance_id = $instance; error = (Short $_.Exception.Message) } }
        try {
            $instance = [string](Get-PnpDeviceProperty -InstanceId $instance -KeyName 'DEVPKEY_Device_Parent' -ErrorAction Stop).Data
        } catch {
            Emit 'parent_query_error' @{ depth = $depth; error = (Short $_.Exception.Message) }
            break
        }
    }
} catch { Emit 'device_query_error' @{ error = (Short $_.Exception.Message) } }
Emit 'collection_complete' @{ disk = $request.disk }
