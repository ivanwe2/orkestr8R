// A tiny console app for the process tests (T07):
//   sleep SECONDS   wait, then exit 0
//   print LINES     print "line 1" .. "line N"
//   exit CODE       exit with CODE
//   env NAME        print the variable's value (nothing when unset)

if (args.Length != 2)
{
    Console.Error.WriteLine("usage: testtool sleep SECONDS | print LINES | exit CODE | env NAME");
    return 2;
}

switch (args[0])
{
    case "sleep":
        Thread.Sleep(TimeSpan.FromSeconds(double.Parse(args[1], System.Globalization.CultureInfo.InvariantCulture)));
        return 0;
    case "print":
        for (var i = 1; i <= int.Parse(args[1]); i++)
            Console.WriteLine($"line {i}");
        return 0;
    case "exit":
        return int.Parse(args[1]);
    case "env":
        Console.Write(Environment.GetEnvironmentVariable(args[1]) ?? "");
        return 0;
    default:
        Console.Error.WriteLine($"unknown command: {args[0]}");
        return 2;
}
