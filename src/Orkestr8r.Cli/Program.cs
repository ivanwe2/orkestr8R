namespace Orkestr8r.Cli;

public static class Program
{
    public static async Task<int> Main(string[] args)
    {
        using var cts = new CancellationTokenSource();
        Console.CancelKeyPress += (_, e) => { e.Cancel = true; cts.Cancel(); };

        return await CliApp.RunAsync(args, Console.Out, Console.Error, Environment.CurrentDirectory,
            Environment.GetEnvironmentVariables(), cts.Token);
    }
}
