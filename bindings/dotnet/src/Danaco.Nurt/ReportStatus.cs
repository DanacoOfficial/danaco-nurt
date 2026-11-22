namespace Danaco.Nurt;

/// <summary>Outcome of a program, the field <c>status</c> of the report <c>nurt-report/1</c>.</summary>
public enum ReportStatus
{
    /// <summary>A value introduced by a later version of the report format.</summary>
    Unknown = 0,

    /// <summary>The program completed (<c>ok</c>).</summary>
    Ok = 1,

    /// <summary>The effect was computed without changes (<c>planned</c>).</summary>
    Planned = 2,

    /// <summary>The program continues in the background (<c>running</c>).</summary>
    Running = 3,

    /// <summary>Part of the changes was applied (<c>partial</c>).</summary>
    Partial = 4,

    /// <summary>The changes were rolled back (<c>rolled_back</c>).</summary>
    RolledBack = 5,

    /// <summary>The program failed (<c>failed</c>).</summary>
    Failed = 6,
}
