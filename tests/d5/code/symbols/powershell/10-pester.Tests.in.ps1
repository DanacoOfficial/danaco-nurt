function Get-Fixture {
    "fixture"
}

Describe "Get-Fixture" {
    It "returns text" {
        Get-Fixture | Should -Be "fixture"
    }
}
