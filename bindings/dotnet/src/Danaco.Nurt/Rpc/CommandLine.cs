using System.Collections.Generic;
using System.Diagnostics;
using System.Text;

namespace Danaco.Nurt;

internal static class CommandLine
{
    private static readonly char[] QuotedCharacters = { ' ', '\t', '\n', '\v', '"' };

    public static void AddArguments(ProcessStartInfo info, IReadOnlyList<string> arguments)
    {
#if NET8_0_OR_GREATER
        foreach (string argument in arguments)
        {
            info.ArgumentList.Add(argument);
        }
#else
        var line = new StringBuilder();
        foreach (string argument in arguments)
        {
            if (line.Length > 0)
            {
                line.Append(' ');
            }

            AppendQuoted(line, argument);
        }

        info.Arguments = line.ToString();
#endif
    }

    /// <summary>
    /// Quotes one argument by the rules of <c>CommandLineToArgvW</c>, which the .NET runtime
    /// also applies when it splits <see cref="ProcessStartInfo.Arguments"/> on Unix.
    /// </summary>
    internal static void AppendQuoted(StringBuilder line, string argument)
    {
        if (argument.Length > 0 && argument.IndexOfAny(QuotedCharacters) < 0)
        {
            line.Append(argument);
            return;
        }

        line.Append('"');
        int backslashes = 0;
        foreach (char c in argument)
        {
            if (c == '\\')
            {
                backslashes++;
                continue;
            }

            line.Append('\\', c == '"' ? (backslashes * 2) + 1 : backslashes);
            line.Append(c);
            backslashes = 0;
        }

        line.Append('\\', backslashes * 2);
        line.Append('"');
    }
}
