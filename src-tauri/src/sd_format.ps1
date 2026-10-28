# Embedded in the signed application; requests arrive as JSON on stdin.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$WarningPreference = 'SilentlyContinue'
[Console]::InputEncoding = New-Object System.Text.UTF8Encoding($false)
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$request = [Console]::In.ReadToEnd() | ConvertFrom-Json
function Send-Event($value) { [Console]::WriteLine(($value | ConvertTo-Json -Compress -Depth 8)) }
function Stage($name, $message) { Send-Event @{ stage = $name; message = $message } }
function Describe($disk) {
    if ($disk.IsBoot -or $disk.IsSystem -or $disk.IsReadOnly -or $disk.IsOffline) {
        throw 'システム・書込禁止・オフラインのディスクはフォーマットできません。'
    }
    if ([string]::IsNullOrWhiteSpace($disk.Path)) { throw 'SDディスクを識別できません。' }
    return @{
        disk = [uint32]$disk.Number; capacity = [uint64]$disk.Size
        path = [string]$disk.Path; unique_id = [string]$disk.UniqueId
        serial = ([string]$disk.SerialNumber).Trim(); sector_size = [uint32]$disk.LogicalSectorSize
    }
}
function Current-Disk($expected, [bool]$checkUniqueId = $false) {
    $disk = Get-Disk -Path $expected.path -ErrorAction Stop
    $current = Describe $disk
    foreach ($field in @('disk', 'capacity', 'path', 'serial', 'sector_size')) {
        if ($current[$field] -cne $expected.$field) { throw '対象SDカードが変更されました。選択し直してください。' }
    }
    if ($checkUniqueId -and $current.unique_id -cne $expected.unique_id) { throw '対象SDカードの識別情報が変更されました。' }
    return $disk
}

try {
    Import-Module Storage -ErrorAction Stop
    foreach ($name in @('Get-Disk', 'Get-Partition', 'Clear-Disk', 'Initialize-Disk', 'New-Partition', 'Format-Volume', 'Get-Volume', 'Add-PartitionAccessPath')) {
        $null = Get-Command $name -ErrorAction Stop
    }
    if ($request.mode -eq 'inspect') {
        $disk = Get-Disk -Number ([uint32]$request.disk) -ErrorAction Stop
        Send-Event @{ result = (Describe $disk) }
        return
    }
    if ($request.mode -ne 'format') { throw 'フォーマット要求が不正です。' }
    $expected = $request.target
    $disk = Current-Disk $expected $true
    # Repeat policy checks inside the process performing the destructive work.
    if ($expected.sector_size -ne 512 -or $expected.capacity -lt 16MB -or $expected.capacity -gt 2TB) {
        throw '対応容量は16MiB〜2TiB、論理セクターサイズは512バイトです。'
    }
    $fs = if ($expected.capacity -le 2GB) { 'FAT' } elseif ($expected.capacity -le 32GB) { 'FAT32' } else { 'exFAT' }
    if ($request.file_system -cne $fs) { throw '容量とファイルシステムが一致しません。' }
    $label = [string]$request.label
    if ($label.Length -gt 11 -or $label -match '[^A-Za-z0-9 _-]') {
        throw 'ラベルは半角英数字・空白・_・-で11文字以内にしてください。'
    }
    # Refuse active pagefile disks before removing any partition information.
    $letters = @()
    $existingPartitions = @()
    if ([string]$disk.PartitionStyle -ne 'RAW' -and $disk.NumberOfPartitions -gt 0) {
        $existingPartitions = @(Get-Partition -DiskNumber $expected.disk -ErrorAction Stop)
        $letters = @($existingPartitions | Where-Object DriveLetter | ForEach-Object { [string]$_.DriveLetter })
    }
    foreach ($page in @(Get-CimInstance Win32_PageFileUsage -ErrorAction Stop)) {
        if ($letters -contains $page.Name.Substring(0, 1)) { throw 'ページファイルを含むディスクはフォーマットできません。' }
    }
    Stage 'format_partition' 'パーティションを作り直しています'
    $disk = Current-Disk $expected $true
    # Windows exposes some removable media as one whole-device volume at offset 0.
    # Such a device already has a format target; rebuilding its partition table is not supported.
    $wholeDeviceVolume = $existingPartitions.Count -eq 1 -and $existingPartitions[0].Offset -eq 0 -and $existingPartitions[0].Size -eq $expected.capacity
    if ($wholeDeviceVolume) {
        $partition = $existingPartitions[0]
    } else {
    if ([string]$disk.PartitionStyle -ne 'RAW') {
        Clear-Disk -InputObject $disk -RemoveData -RemoveOEM -Confirm:$false -ErrorAction Stop
    }
    $disk = Current-Disk $expected
    # Some removable media remain initialized as MBR after Clear-Disk.
    # Initialize-Disk accepts RAW disks only; do not reinitialize an existing MBR.
    if ([string]$disk.PartitionStyle -eq 'RAW') {
        $null = Initialize-Disk -InputObject $disk -PartitionStyle MBR -PassThru -ErrorAction Stop
    }
    $disk = Current-Disk $expected
    if ([string]$disk.PartitionStyle -ne 'MBR') { throw 'MBR形式のディスクを準備できませんでした。' }
    $partitionArguments = @{ DiskNumber = $expected.disk; UseMaximumSize = $true; Alignment = 1MB; ErrorAction = 'Stop' }
    $partitionArguments.MbrType = if ($fs -eq 'FAT') { 'Huge' } elseif ($fs -eq 'FAT32') { 'FAT32' } else { 'IFS' }
    $preferred = if ([string]$request.drive -cmatch '^[A-Z]:$') { ([string]$request.drive).Substring(0, 1) } else { '' }
    if ($preferred -and -not (Get-Volume | Where-Object { [string]$_.DriveLetter -eq $preferred })) {
        $partitionArguments.DriveLetter = [char]$preferred
    } else { $partitionArguments.AssignDriveLetter = $true }
    $partition = New-Partition @partitionArguments
    }
    $null = Current-Disk $expected
    Stage 'format_volume' 'クイックフォーマットしています'
    $volume = Format-Volume -Partition $partition -FileSystem $fs -NewFileSystemLabel $label -Force -Confirm:$false -ErrorAction Stop
    $null = Current-Disk $expected
    $partition = Get-Partition -DiskNumber $expected.disk -PartitionNumber $partition.PartitionNumber -ErrorAction Stop
    if (-not $partition.DriveLetter) {
        Add-PartitionAccessPath -InputObject $partition -AssignDriveLetter -ErrorAction Stop
        $partition = Get-Partition -DiskNumber $expected.disk -PartitionNumber $partition.PartitionNumber -ErrorAction Stop
    }
    Stage 'format_verify' 'フォーマット結果を確認しています'
    $partitions = @(Get-Partition -DiskNumber $expected.disk -ErrorAction Stop)
    $volume = Get-Volume -Partition $partition -ErrorAction Stop
    if ($partitions.Count -ne 1 -or $volume.FileSystem -ine $fs -or $volume.FileSystemLabel -cne $label -or
        -not $volume.DriveLetter -or $volume.Size -le 0 -or $volume.Size -gt $expected.capacity) {
        throw '作成したパーティション・ファイルシステムを確認できませんでした。'
    }
    if ([string](Get-Disk -Path $expected.path).PartitionStyle -ne 'MBR') { throw 'MBRを確認できませんでした。' }
    $root = '{0}:\' -f $volume.DriveLetter
    $testPath = Join-Path $root ('.ts-sddirecta-format-' + [guid]::NewGuid().ToString('N') + '.tmp')
    $bytes = [Text.Encoding]::UTF8.GetBytes('ts-sddirecta format verification ' + [guid]::NewGuid())
    $file = New-Object System.IO.FileStream($testPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $file.Write($bytes, 0, $bytes.Length); $file.Flush($true) } finally { $file.Dispose() }
    $actual = [IO.File]::ReadAllBytes($testPath)
    if ([Convert]::ToBase64String($actual) -cne [Convert]::ToBase64String($bytes)) { throw 'テストファイルの読戻しが一致しません。' }
    [IO.File]::Delete($testPath)
    if ([IO.File]::Exists($testPath)) { throw 'テストファイルを削除できません。' }
    $null = Current-Disk $expected
    Send-Event @{ result = @{ drive = ('{0}:' -f $volume.DriveLetter); file_system = [string]$volume.FileSystem; capacity = [uint64]$volume.Size; label = [string]$volume.FileSystemLabel } }
} catch {
    Send-Event @{ error = $_.Exception.Message }
    exit 1
}
