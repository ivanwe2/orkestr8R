namespace Orkestr8r.Cli;

/// <summary>The process exit codes. A contract: see docs/cli.md.</summary>
public static class ExitCodes
{
    public const int Ok = 0;
    public const int Error = 1;
    /// <summary>Waiting for a human. Reserved in v0.1.</summary>
    public const int Waiting = 3;
    /// <summary>Finished, but the checks failed or the grunt gave up.</summary>
    public const int ChecksFailed = 4;
}
