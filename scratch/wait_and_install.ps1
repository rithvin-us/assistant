$apkPath = "apps\mobile\src-tauri\gen\android\app\build\outputs\apk\arm64\debug\app-arm64-debug.apk"
Write-Host "Waiting for Android device to connect via ADB..."

while ($true) {
    $devices = adb devices | Select-String -Pattern "\tdevice$"
    if ($devices) {
        $deviceId = ($devices[0] -split "\t")[0]
        Write-Host "Device detected: $deviceId. Installing $apkPath..."
        $installResult = adb -s $deviceId install -r $apkPath
        Write-Host $installResult
        Write-Host "Launching com.rithvin.assistant..."
        adb -s $deviceId shell monkey -p com.rithvin.assistant -c android.intent.category.LAUNCHER 1
        Write-Host "Installation and launch complete!"
        break
    }
    Start-Sleep -Seconds 3
}
