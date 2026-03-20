try {

$exePath = "C:\Users\h2art\AppData\Local\Programs\PrismLauncher\prismlauncher.exe"

Write-Host "=== PrismLauncher 상태 확인 ===" -ForegroundColor Cyan
Write-Host "대상 경로: $exePath"
Write-Host ""

# PrismLauncher 프로세스 찾기
$prism = Get-Process | Where-Object { $_.Path -eq $exePath } | Select-Object -First 1

if ($null -eq $prism) {
    Write-Host "[결과] PrismLauncher 꺼져있음" -ForegroundColor Yellow
    Write-Host "-> 모드팩 가져오기 후 PrismLauncher를 시작합니다"
    exit
}

Write-Host "[PrismLauncher] PID: $($prism.Id)" -ForegroundColor Green

# java 자식 프로세스 확인
$java = Get-CimInstance Win32_Process | Where-Object {
    $_.ParentProcessId -eq $prism.Id -and $_.Name -match 'java'
} | Select-Object -First 1

if ($null -eq $java) {
    Write-Host "[결과] 게임 미실행 (java 없음)" -ForegroundColor Yellow
    Write-Host "-> 모드팩 가져오기 후 PrismLauncher를 재시작합니다"
} else {
    Write-Host "[java] PID: $($java.ProcessId), 이름: $($java.Name)" -ForegroundColor Green
    Write-Host "[결과] 게임 실행 중 (녹화 보호)" -ForegroundColor Red
    Write-Host "-> 모드팩만 풀어놓고 재시작하지 않습니다"
}

} catch {
    Write-Host "[오류] $_" -ForegroundColor Red
}

Write-Host ""
Write-Host "아무 키나 누르면 종료합니다..."
$null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
