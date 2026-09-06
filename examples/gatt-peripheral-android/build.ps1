param([switch]$Run)
$ErrorActionPreference = 'Stop'
if (-not $env:JAVA_HOME) {
    $studioJava = Join-Path $env:ProgramFiles 'Android/Android Studio/jbr'
    if (Test-Path "$studioJava/bin/java.exe") { $env:JAVA_HOME = $studioJava }
    else { throw 'Set JAVA_HOME to a JDK 17 or newer.' }
}
if (-not $env:ANDROID_HOME) {
    $env:ANDROID_HOME = Join-Path $env:LOCALAPPDATA 'Android/Sdk'
}
& "$PSScriptRoot/gradlew.bat" -p $PSScriptRoot assembleDebug --console=plain
if ($LASTEXITCODE -ne 0) { throw 'Gradle build failed' }
if ($Run) {
    $androidCli = Get-Command android -ErrorAction SilentlyContinue
    $androidExe = if ($androidCli) { $androidCli.Source } else { Join-Path $env:USERPROFILE 'AppData/AndroidCLI/android.exe' }
    & $androidExe run "--apks=$PSScriptRoot/app/build/outputs/apk/debug/app-debug.apk"
    if ($LASTEXITCODE -ne 0) { throw 'Android deployment failed' }
}
