class Service {
    [string]$Name

    [void] Start() {}
}

function Start-App {
    [Service]::new().Start()
}
