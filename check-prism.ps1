param(
    [Parameter(Mandatory = $true)][string]$ExePath,
    [string]$FixturePath
)

$ErrorActionPreference = 'Stop'
try {
    $normalizedExe = [System.IO.Path]::GetFullPath($ExePath)
    if ($FixturePath) {
        $fixture = Get-Content -LiteralPath $FixturePath -Raw -Encoding UTF8 | ConvertFrom-Json
        $processes = @($fixture.processes)
        $children = @($fixture.children)
    } else {
        $processes = @(Get-Process | ForEach-Object {
            try {
                if ($_.Path) { [pscustomobject]@{ Id = $_.Id; Path = $_.Path } }
            } catch {
                # 접근 권한이 없는 다른 프로세스의 경로는 비교할 수 없다.
            }
        })
        $children = @(Get-CimInstance Win32_Process)
    }

    Write-Output "PrismLauncher 경로: $normalizedExe"
    $prism = $processes | Where-Object {
        $_.Path -and [string]::Equals(
            [System.IO.Path]::GetFullPath([string]$_.Path),
            $normalizedExe,
            [System.StringComparison]::OrdinalIgnoreCase
        )
    } | Select-Object -First 1

    if ($null -eq $prism) {
        Write-Output '상태: 미실행'
        Write-Output '계획: 가져오기 후에도 런처를 꺼진 상태로 유지'
        return
    }

    Write-Output "PrismLauncher PID: $($prism.Id)"
    $game = $children | Where-Object {
        $_.ParentProcessId -eq $prism.Id -and $_.Name -match '^java(w)?\.exe$'
    } | Select-Object -First 1

    if ($null -ne $game) {
        Write-Output "게임 PID: $($game.ProcessId)"
        Write-Output '상태: 게임 실행 중'
        Write-Output '계획: 가져오기를 보류하고 기존 파일을 변경하지 않음'
    } else {
        Write-Output '상태: 런처 실행 중, 게임 미실행'
        Write-Output '계획: 런처 정상 종료를 확인한 후 가져오고, 성공하면 재실행'
    }
} catch {
    Write-Error "프로세스 상태 조회 실패: $_"
    exit 1
}
