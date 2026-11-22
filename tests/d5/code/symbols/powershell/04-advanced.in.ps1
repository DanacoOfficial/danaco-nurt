function Invoke-Task {
    [CmdletBinding()]
    param([Parameter(Mandatory)] [string]$Name)
    process {
        $Name
    }
}
