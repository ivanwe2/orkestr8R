using System.Collections;
using System.CommandLine;

namespace Orkestr8r.Cli;

/// <summary>
/// The whole CLI, callable in-process: <see cref="Program.Main"/> only forwards to
/// <see cref="RunAsync"/>, so tests run commands with their own writers, directory and environment.
/// </summary>
public static class CliApp
{
    public static RootCommand BuildRoot()
    {
        var root = new RootCommand("orkestr8R: run coding orders on a local model, in git worktrees, checked with real commands.");
        return root;
    }

    public static async Task<int> RunAsync(string[] args, TextWriter stdout, TextWriter stderr, string cwd,
        IDictionary env, CancellationToken ct)
    {
        var root = BuildRoot();
        var parse = root.Parse(args);
        var invocation = new InvocationConfiguration
        {
            Output = stdout,
            Error = stderr,
            // Program.Main wires Ctrl+C to ct itself.
            ProcessTerminationTimeout = null,
        };
        return await parse.InvokeAsync(invocation, ct);
    }
}
