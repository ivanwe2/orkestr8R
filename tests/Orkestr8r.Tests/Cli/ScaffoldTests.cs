using Orkestr8r.Cli;

namespace Orkestr8r.Tests.Cli;

public class ScaffoldTests
{
    [Fact]
    public async Task Version_starts_with_0_1_0()
    {
        var stdout = new StringWriter();
        var stderr = new StringWriter();

        var exit = await CliApp.RunAsync(["--version"], stdout, stderr, Environment.CurrentDirectory,
            new Dictionary<string, string>(), TestContext.Current.CancellationToken);

        Assert.Equal(ExitCodes.Ok, exit);
        Assert.StartsWith("0.1.0", stdout.ToString());
    }
}
