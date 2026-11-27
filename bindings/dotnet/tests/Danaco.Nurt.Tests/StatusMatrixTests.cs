// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/tests/Danaco.Nurt.Tests/StatusMatrixTests.cs
// Responsibility:   Source file bindings/dotnet/tests/Danaco.Nurt.Tests/StatusMatrixTests.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using System.Threading.Tasks;
using Microsoft.VisualStudio.TestTools.UnitTesting;

namespace Danaco.Nurt.Tests;

/// <summary>
/// Matrix of the functions of <c>nurt.h</c> and the statuses each may return, read from the
/// table "Possible status codes of each function" of the header, against the declarations and
/// the error mapping of the binding.
/// </summary>
[TestClass]
public sealed partial class StatusMatrixTests
{
    private const string WaitFunction = "nurt_exec_wait";
    private const string SchemaFunction = "nurt_schema";
    private const string PrintFunction = "nurt_print";

    private static readonly string[] WaitOnly = { WaitFunction };
    private static readonly string[] HeaderPath = { "crates", "nurt-ffi", "include", "nurt.h" };
    private static readonly string[] LibraryImportPath = { "bindings", "dotnet", "src", "Danaco.Nurt", "Interop", "NativeMethods.LibraryImport.cs" };
    private static readonly string[] DllImportPath = { "bindings", "dotnet", "src", "Danaco.Nurt", "Interop", "NativeMethods.DllImport.cs" };

    [TestMethod]
    public void StatusValuesAndNamesMatchTheHeader()
    {
        HeaderTable header = HeaderTable.Read();

        foreach (KeyValuePair<string, int> status in header.Statuses)
        {
            Assert.IsTrue(Enum.IsDefined((CallStatus)status.Value), status.Key + " has no member in CallStatus");
            Assert.AreEqual(status.Key, NurtCallException.GetStatusName((CallStatus)status.Value));
        }

        Assert.AreEqual(header.Statuses.Count, Enum.GetValues<CallStatus>().Length);
    }

    [TestMethod]
    public void EveryColumnOfTheTableIsAStatusOfTheHeader()
    {
        HeaderTable header = HeaderTable.Read();

        foreach (string column in header.Columns)
        {
            Assert.IsTrue(header.Statuses.ContainsKey(column), column + " is not a status of nurt_status");
        }

        Assert.AreEqual(header.Statuses.Count, header.Columns.Count);
    }

    [TestMethod]
    public void EveryFunctionOfTheTableCanSucceed()
    {
        HeaderTable header = HeaderTable.Read();

        Assert.IsTrue(header.Rows.Count > 0, "the status table of nurt.h is empty");
        foreach (KeyValuePair<string, HashSet<string>> row in header.Rows)
        {
            Assert.IsTrue(row.Value.Contains("NURT_OK"), row.Key + " never returns NURT_OK");
        }
    }

    [TestMethod]
    public void OnlyExecWaitReportsTimeoutAndCancellation()
    {
        HeaderTable header = HeaderTable.Read();
        string timeout = NurtCallException.GetStatusName(CallStatus.Timeout);
        string cancelled = NurtCallException.GetStatusName(CallStatus.Cancelled);

        string[] waiting = header.Rows
            .Where(row => row.Value.Contains(timeout) || row.Value.Contains(cancelled))
            .Select(row => row.Key)
            .ToArray();

        CollectionAssert.AreEqual(WaitOnly, waiting);
        Assert.IsTrue(header.Rows[WaitFunction].Contains(timeout));
        Assert.IsTrue(header.Rows[WaitFunction].Contains(cancelled));
    }

    [TestMethod]
    public void ImportedFunctionsAreDeclaredInTheHeaderWithTheirReturnKind()
    {
        HeaderTable header = HeaderTable.Read();

        foreach (string[] path in new[] { LibraryImportPath, DllImportPath })
        {
            Dictionary<string, string> imports = Imports(path);
            Assert.IsTrue(imports.Count > 0, "no imports found in " + Path.GetFileName(path[^1]));
            foreach (KeyValuePair<string, string> import in imports)
            {
                bool returnsStatus = header.Rows.ContainsKey(import.Key);
                Assert.IsTrue(returnsStatus || header.WithoutStatus.Contains(import.Key), import.Key + " is not declared in nurt.h");
                Assert.AreEqual(returnsStatus, import.Value == nameof(CallStatus), import.Key + " returns " + import.Value);
            }
        }
    }

    [TestMethod]
    public void BothDeclarationsImportTheSameFunctions()
    {
        string[] generated = Imports(LibraryImportPath).Keys.OrderBy(name => name, StringComparer.Ordinal).ToArray();
        string[] classic = Imports(DllImportPath).Keys.OrderBy(name => name, StringComparer.Ordinal).ToArray();

        CollectionAssert.AreEqual(generated, classic);
    }

    [TestMethod]
    public void ProtocolErrorsCarryEveryStatusOfTheHeader()
    {
        HeaderTable header = HeaderTable.Read();

        foreach (KeyValuePair<string, int> status in header.Statuses.Where(status => status.Value != (int)CallStatus.Ok))
        {
            CallStatus mapped = RpcErrors.ToStatus(RpcErrors.StatusBase - status.Value);
            Assert.AreEqual(status.Key, NurtCallException.GetStatusName(mapped));
        }
    }

    [TestMethod]
    public async Task LibraryReturnsOnlyStatusesOfTheTable()
    {
        string library = TestSupport.Variable(TestSupport.LibraryVariable)
            ?? throw new AssertInconclusiveException(TestSupport.LibraryVariable + " does not name a build of libnurt");
        HeaderTable header = HeaderTable.Read();
        await using var client = new NurtClient(new NurtClientOptions { Backend = BackendKind.Library, LibraryPath = library });

        NurtCallException unknownSchema = await TestSupport.ThrowsAsync<NurtCallException>(() => client.SchemaAsync("unknown"));
        NurtCallException invalidTree = await TestSupport.ThrowsAsync<NurtCallException>(() => client.PrintAsync(TestSupport.Json("[]")));

        Assert.AreEqual(CallStatus.InvalidArgument, unknownSchema.Status);
        Assert.IsTrue(header.Rows[SchemaFunction].Contains(unknownSchema.StatusName));
        Assert.AreEqual(CallStatus.InvalidArgument, invalidTree.Status);
        Assert.IsTrue(header.Rows[PrintFunction].Contains(invalidTree.StatusName));
    }

    /// <summary>Entry points of the declarations in a source file and the return type of each.</summary>
    private static Dictionary<string, string> Imports(string[] path)
    {
        string source = File.ReadAllText(TestSupport.RequireRepositoryFile(path));
        var imports = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (Match match in ImportPattern().Matches(source))
        {
            imports.Add(match.Groups["entry"].Value, match.Groups["type"].Value);
        }

        return imports;
    }

    [GeneratedRegex("EntryPoint = \"(?<entry>nurt_[a-z_]+)\".*?internal static (?:extern|partial) (?<type>[A-Za-z]+) ", RegexOptions.Singleline | RegexOptions.CultureInvariant)]
    private static partial Regex ImportPattern();

    [GeneratedRegex(@"^\s*(?<name>NURT_(?:OK|E_[A-Z]+))\s*=\s*(?<value>[0-9]+)", RegexOptions.Multiline | RegexOptions.CultureInvariant)]
    private static partial Regex StatusPattern();

    [GeneratedRegex("nurt_[a-z_]+", RegexOptions.CultureInvariant)]
    private static partial Regex FunctionPattern();

    [GeneratedRegex("[A-Z]+", RegexOptions.CultureInvariant)]
    private static partial Regex ColumnPattern();

    /// <summary>Status table, status values and functions without a status, as written in <c>nurt.h</c>.</summary>
    private sealed class HeaderTable
    {
        private const string TableHeading = "function";
        private const string WithoutStatusEnd = "do not return a status";

        private HeaderTable(Dictionary<string, int> statuses, List<string> columns, Dictionary<string, HashSet<string>> rows, HashSet<string> withoutStatus)
        {
            Statuses = statuses;
            Columns = columns;
            Rows = rows;
            WithoutStatus = withoutStatus;
        }

        /// <summary>Names of <c>nurt_status</c> and their values.</summary>
        public Dictionary<string, int> Statuses { get; }

        /// <summary>Statuses named by the columns of the table, in the order of the columns.</summary>
        public List<string> Columns { get; }

        /// <summary>Function of each row and the statuses marked for it.</summary>
        public Dictionary<string, HashSet<string>> Rows { get; }

        /// <summary>Functions that the header lists as returning no status.</summary>
        public HashSet<string> WithoutStatus { get; }

        /// <summary>
        /// Reads the header. The table is aligned in columns: a mark <c>x</c> stands under the
        /// first letter of the name of its column.
        /// </summary>
        public static HeaderTable Read()
        {
            string text = File.ReadAllText(TestSupport.RequireRepositoryFile(HeaderPath));
            var statuses = new Dictionary<string, int>(StringComparer.Ordinal);
            foreach (Match match in StatusPattern().Matches(text))
            {
                statuses.Add(match.Groups["name"].Value, int.Parse(match.Groups["value"].ValueSpan, NumberStyles.None, CultureInfo.InvariantCulture));
            }

            string[] lines = text.Split('\n');
            int heading = Array.FindIndex(lines, line => Content(line).StartsWith(TableHeading + " ", StringComparison.Ordinal));
            Assert.IsTrue(heading >= 0, "nurt.h has no status table");

            var columns = new List<(int Start, int End, string Status)>();
            foreach (Match column in ColumnPattern().Matches(lines[heading]))
            {
                string status = column.Value == "OK" ? "NURT_OK" : "NURT_E_" + column.Value;
                columns.Add((column.Index, column.Index + column.Length, status));
            }

            var rows = new Dictionary<string, HashSet<string>>(StringComparer.Ordinal);
            int index = heading + 1;
            for (; index < lines.Length; index++)
            {
                Match function = FunctionPattern().Match(lines[index]);
                string afterName = function.Success ? lines[index][(function.Index + function.Length)..] : string.Empty;
                if (!function.Success || afterName.Contains(',', StringComparison.Ordinal) || !afterName.Contains('x', StringComparison.Ordinal))
                {
                    break;
                }

                var marked = new HashSet<string>(StringComparer.Ordinal);
                for (int position = function.Index + function.Length; position < lines[index].Length; position++)
                {
                    if (lines[index][position] != 'x')
                    {
                        continue;
                    }

                    (int Start, int End, string Status) owner = columns.LastOrDefault(column => column.Start <= position);
                    Assert.IsTrue(owner.Status is not null && position < owner.End, "a mark of " + function.Value + " stands outside the columns");
                    marked.Add(owner.Status!);
                }

                rows.Add(function.Value, marked);
            }

            var withoutStatus = new HashSet<string>(StringComparer.Ordinal);
            for (; index < lines.Length; index++)
            {
                foreach (Match function in FunctionPattern().Matches(lines[index]))
                {
                    withoutStatus.Add(function.Value);
                }

                if (lines[index].Contains(WithoutStatusEnd, StringComparison.Ordinal))
                {
                    break;
                }
            }

            return new HeaderTable(statuses, columns.Select(column => column.Status).ToList(), rows, withoutStatus);
        }

        /// <summary>Text of a line of the block comment without the leading <c>*</c>.</summary>
        private static string Content(string line) => line.TrimStart().TrimStart('*').TrimStart();
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
