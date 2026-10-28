# Test double for Windows Storage. Never imports Storage or calls a real disk cmdlet.
param([string]$Mode, [string]$Root)
$ErrorActionPreference = 'Stop'
function Import-Module { param($Name, $ErrorAction) }
function Call($name) { [Console]::Error.WriteLine("CALL:$name") }
$script:reads = 0
function Get-Disk {
    param($Number, $Path, $ErrorAction)
    Call 'Get-Disk'
    $script:reads++
    $serial = if ($Mode -eq 'changed' -and $script:reads -gt 1) { 'other-card' } else { 'test-card' }
    return [pscustomobject]@{
        Number = 77; Path = 'test-device'; UniqueId = 'test-id'; SerialNumber = $serial
        NumberOfPartitions = $(if ($script:cleared -or $Mode -eq 'raw') { 0 } else { 1 })
        Size = 536870912; LogicalSectorSize = 512; IsBoot = ($Mode -eq 'system'); IsSystem = $false
        IsOffline = $false; IsReadOnly = ($Mode -eq 'readonly'); PartitionStyle = $(if (($script:cleared -and $Mode -ne 'keep-mbr') -or ($Mode -eq 'raw' -and -not $script:initialized)) { 'RAW' } else { 'MBR' })
    }
}
function Get-Partition {
    param($DiskNumber, $PartitionNumber, $ErrorAction)
    Call 'Get-Partition'
    return [pscustomobject]@{ DriveLetter = $(if ($Mode -eq 'unassigned' -and -not $script:assigned) { '' } else { 'H' }); DiskNumber = 77; PartitionNumber = 1
        Offset = $(if ($Mode -eq 'unassigned') { 0 } else { 1048576 }); Size = $(if ($Mode -eq 'unassigned') { 536870912 } else { 530000000 }) }
}
function Get-CimInstance { param($ClassName, $ErrorAction); return @() }
function Clear-Disk {
    param($InputObject, [switch]$RemoveData, [switch]$RemoveOEM, $Confirm, $ErrorAction)
    Call 'Clear-Disk'
    if ($InputObject.Number -ne 77) { throw 'Wrong disk' }
    $script:cleared = $true
}
function Initialize-Disk {
    param($InputObject, $PartitionStyle, [switch]$PassThru, $ErrorAction)
    Call 'Initialize-Disk'
    if ($InputObject.PartitionStyle -ne 'RAW') { throw 'The disk has already been initialized.' }
    if ($PartitionStyle -ne 'MBR') { throw 'Expected MBR' }
    $script:cleared = $false
    $script:initialized = $true
    return $InputObject
}
function New-Partition {
    param($DiskNumber, [switch]$UseMaximumSize, $Alignment, $MbrType, $DriveLetter, [switch]$AssignDriveLetter, $ErrorAction)
    Call 'New-Partition'
    if ($DiskNumber -ne 77 -or -not $UseMaximumSize -or $MbrType -ne 'Huge') { throw 'Wrong partition parameters' }
    return [pscustomobject]@{ DiskNumber = 77; PartitionNumber = 1; DriveLetter = 'H' }
}
function Add-PartitionAccessPath {
    param($InputObject, [switch]$AssignDriveLetter, $ErrorAction)
    Call 'Add-PartitionAccessPath'
    if ($InputObject.DiskNumber -ne 77) { throw 'Wrong disk' }
    $script:assigned = $true
}
function Format-Volume {
    param($Partition, $FileSystem, $NewFileSystemLabel, [switch]$Force, $Confirm, $ErrorAction)
    Call 'Format-Volume'
    if ($Mode -eq 'format-error') { throw 'Simulated formatter error' }
    if ($Partition.DiskNumber -ne 77 -or $FileSystem -ne 'FAT') { throw 'Wrong volume' }
    $script:formatted = $true
}
function Get-Volume {
    param($Partition, $ErrorAction)
    Call 'Get-Volume'
    if (-not $Partition) { return @() }
    return [pscustomobject]@{
        DriveLetter = 'H'; FileSystem = $(if ($Mode -eq 'bad-result') { 'NTFS' } else { 'FAT' })
        FileSystemLabel = 'TEST'; Size = 530000000
    }
}
$source = [IO.File]::ReadAllText((Join-Path (Split-Path -Parent $PSScriptRoot) 'src-tauri/src/sd_format.ps1'))
# Only the file IO check is redirected into a disposable folder; all storage calls above are mocked.
$source = $source.Replace("`$root = '{0}:\' -f `$volume.DriveLetter", '`$root = $Root'.Replace('`$', '$'))
& ([scriptblock]::Create($source))
