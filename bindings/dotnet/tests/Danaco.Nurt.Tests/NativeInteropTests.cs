// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.VisualStudio.TestTools.UnitTesting;

namespace Danaco.Nurt.Tests;

/// <summary>
/// The interop layer without the native library: the declarations against the prototypes of
/// <c>nurt.h</c>, the decoding of <c>nurt_buf</c> and the receiver of <c>nurt_event_cb</c>.
/// </summary>
[TestClass]
public sealed unsafe partial class NativeInteropTests
{
    private static readonly string[] HeaderPath = { "crates", "nurt-ffi", "include", "nurt.h" };
    private static readonly string[] LibraryImportPath = { "bindings", "dotnet", "src", "Danaco.Nurt", "Interop", "NativeMethods.LibraryImport.cs" };
    private static readonly string[] DllImportPath = { "bindings", "dotnet", "src", "Danaco.Nurt", "Interop", "NativeMethods.DllImport.cs" };

    private static readonly string[] ExpectedKinds = { "progress", "exec.finished" };
    private static readonly long[] ExpectedSequences = { 1, 2 };
    private static readonly byte[] EmojiTerminated = { 0xF0, 0x9F, 0x98, 0x80, 0 };
    private static readonly byte[] TerminatorOnly = { 0 };
    private static readonly byte[] BracesTerminated = { 0x7B, 0x7D, 0 };
    private static readonly byte[] Braces = { 0x7B, 0x7D };
    private static readonly byte[] InvalidText = { 0x61, 0xFF, 0xFE, 0 };
    private static readonly byte[] InvalidEvent = { 0x7B, 0xFF, 0xFE, 0x7D };

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void EventDelegate(IntPtr user, IntPtr eventJson);

    [TestMethod]
    public void EveryDeclarationHasTheParametersOfItsPrototype()
    {
        Dictionary<string, (string Ret, string[] Args)> header = Prototypes();
        Dictionary<string, (string Name, string Ret, string[] Args)> imports = Imports(LibraryImportPath);
        Assert.IsTrue(imports.Count > 0, "no declarations found");

        foreach (KeyValuePair<string, (string Name, string Ret, string[] Args)> import in imports)
        {
            Assert.IsTrue(header.ContainsKey(import.Key), import.Key + " is not declared in nurt.h");
            (string Ret, string[] Args) prototype = header[import.Key];
            MethodInfo method = typeof(NativeMethods).GetMethod(import.Value.Name, BindingFlags.Static | BindingFlags.Public | BindingFlags.NonPublic)
                ?? throw new AssertFailedException(import.Value.Name + " is not a member of NativeMethods");
            ParameterInfo[] parameters = method.GetParameters();

            Assert.IsTrue(Accepts(prototype.Ret, method.ReturnType), import.Key + " returns " + method.ReturnType.Name + " but nurt.h declares " + prototype.Ret);
            Assert.AreEqual(prototype.Args.Length, parameters.Length, import.Key + " has another number of parameters than nurt.h");
            for (int index = 0; index < parameters.Length; index++)
            {
                Assert.IsTrue(
                    Accepts(prototype.Args[index], parameters[index].ParameterType),
                    import.Key + " parameter " + index.ToString(CultureInfo.InvariantCulture) + " is " + parameters[index].ParameterType + " but nurt.h declares " + prototype.Args[index]);
            }
        }
    }

    [TestMethod]
    public void BothDeclarationsHaveTheSameSignatures()
    {
        Dictionary<string, (string Name, string Ret, string[] Args)> generated = Imports(LibraryImportPath);
        Dictionary<string, (string Name, string Ret, string[] Args)> classic = Imports(DllImportPath);

        CollectionAssert.AreEquivalent(generated.Keys.ToList(), classic.Keys.ToList());
        foreach (KeyValuePair<string, (string Name, string Ret, string[] Args)> import in generated)
        {
            (string Name, string Ret, string[] Args) other = classic[import.Key];
            Assert.AreEqual(import.Value.Name, other.Name, import.Key);
            Assert.AreEqual(import.Value.Ret, other.Ret, import.Key);
            CollectionAssert.AreEqual(import.Value.Args, other.Args, import.Key);
        }
    }

    [TestMethod]
    public void BufferDecodesItsLengthAndIgnoresWhatFollows()
    {
        const string Text = "zażółć gęślą jaźń";
        byte[] bytes = Encoding.UTF8.GetBytes(Text + "\0tail");

        Assert.AreEqual(Text, ReadString(bytes, Encoding.UTF8.GetByteCount(Text)));
    }

    [TestMethod]
    public void EmptyBufferHasNoTextAndNoJson()
    {
        NativeBuffer buffer = default;

        Assert.IsTrue(buffer.IsEmpty);
        Assert.AreEqual(string.Empty, buffer.ReadString());
        Assert.IsNull(buffer.ReadJson("result"));
    }

    [TestMethod]
    public void BufferWithInvalidUtf8IsRejected()
    {
        Assert.ThrowsException<DecoderFallbackException>(() => ReadString(InvalidText, 3));
    }

    [TestMethod]
    public void BufferJsonIsParsedAndInvalidJsonIsACallFailure()
    {
        JsonElement? value = ReadJson("""{"a": [1, 2]}""");
        NurtCallException failure = Assert.ThrowsException<NurtCallException>(() => ReadJson("{"));

        Assert.AreEqual(2, value!.Value.GetProperty("a").GetArrayLength());
        Assert.AreEqual(CallStatus.Internal, failure.Status);
    }

    [TestMethod]
    public void ZeroTerminatedTextIsDecodedUpToTheTerminator()
    {
        byte[] bytes = Encoding.UTF8.GetBytes("łódź\0ignored\0");

        Assert.AreEqual("łódź", FromTerminated(bytes));
        Assert.AreEqual(string.Empty, Utf8.FromNative(null));
    }

    [TestMethod]
    public void TextForTheLibraryIsStrictUtf8WithTerminator()
    {
        CollectionAssert.AreEqual(EmojiTerminated, Utf8.ToNative("\U0001F600", "text"));
        CollectionAssert.AreEqual(TerminatorOnly, Utf8.ToNative(string.Empty, "text"));
        Assert.ThrowsException<EncoderFallbackException>(() => Utf8.ToNative("a\uD800b", "text"));
        CollectionAssert.AreEqual(BracesTerminated, Utf8.Terminate(Braces));
    }

    [TestMethod]
    public void VersionOfTheHeaderIsMajorShiftedLeftByMinor()
    {
        string header = File.ReadAllText(TestSupport.RequireRepositoryFile(HeaderPath));
        int major = int.Parse(MajorPattern().Match(header).Groups[1].Value, CultureInfo.InvariantCulture);
        int minor = int.Parse(MinorPattern().Match(header).Groups[1].Value, CultureInfo.InvariantCulture);

        Assert.AreEqual(major, NativeMethods.AbiMajor);
        Assert.AreEqual(minor, NativeMethods.AbiMinor);
        Assert.AreEqual((uint)((major << 16) | minor), NativeMethods.AbiHeaderVersion);
        Assert.AreEqual(uint.MaxValue, NativeMethods.WaitForever);
    }

    [TestMethod]
    public async Task EventCallbackQueuesTheEventOfItsExecution()
    {
        var queue = new EventQueue();

        Deliver(queue, Encoding.UTF8.GetBytes("""{"t": "progress", "seq": 1, "time": "2026-01-01T00:00:00Z"}"""));
        Deliver(queue, Encoding.UTF8.GetBytes("""{"t": "exec.finished", "seq": 2, "status": "ok"}"""));
        Deliver(queue, Encoding.UTF8.GetBytes("""{"t": "progress", "seq": 3}"""));

        List<NurtExecutionEvent> events = await ReadAsync(queue);
        CollectionAssert.AreEqual(ExpectedKinds, events.Select(item => item.Kind).ToArray());
        CollectionAssert.AreEqual(ExpectedSequences, events.Select(item => item.Sequence).ToArray());
    }

    [TestMethod]
    public async Task EventCallbackDropsWhatItCannotDecodeWithoutThrowing()
    {
        var queue = new EventQueue();

        Deliver(queue, InvalidEvent);
        Deliver(queue, Encoding.UTF8.GetBytes("not json"));
        Deliver(queue, Encoding.UTF8.GetBytes(string.Empty));
        Deliver(queue, Encoding.UTF8.GetBytes("""{"t": "progress", "seq": 4}"""));

        List<NurtExecutionEvent> events = await ReadAsync(queue);
        Assert.AreEqual(1, events.Count);
        Assert.AreEqual(4, events[0].Sequence);
    }

    [TestMethod]
    public async Task EventCallbackAcceptsAnEmptyEventPointer()
    {
        var queue = new EventQueue();
        EventDelegate callback = Marshal.GetDelegateForFunctionPointer<EventDelegate>(EventCallback.Pointer);
        GCHandle state = GCHandle.Alloc(queue);
        try
        {
            callback(GCHandle.ToIntPtr(state), IntPtr.Zero);
        }
        finally
        {
            state.Free();
        }

        Assert.AreEqual(0, (await ReadAsync(queue)).Count);
    }

    [GeneratedRegex(@"NURT_API\s+(?<ret>[\w\s\*]+?)\s*NURT_CALL\s+(?<name>nurt_\w+)\((?<args>[^)]*)\)\s*;", RegexOptions.CultureInvariant)]
    private static partial Regex PrototypePattern();

    [GeneratedRegex(@"EntryPoint = ""(?<entry>nurt_\w+)"".*?internal static (?:extern|partial) (?<ret>\S+) (?<name>\w+)\((?<args>[^)]*)\);", RegexOptions.CultureInvariant | RegexOptions.Singleline)]
    private static partial Regex ImportPattern();

    [GeneratedRegex(@"^(?<type>.*?)(?<name>\w+)$", RegexOptions.CultureInvariant)]
    private static partial Regex ParameterPattern();

    [GeneratedRegex(@"\s+", RegexOptions.CultureInvariant)]
    private static partial Regex BlankPattern();

    [GeneratedRegex(@"#define NURT_ABI_MAJOR (\d+)", RegexOptions.CultureInvariant)]
    private static partial Regex MajorPattern();

    [GeneratedRegex(@"#define NURT_ABI_MINOR (\d+)", RegexOptions.CultureInvariant)]
    private static partial Regex MinorPattern();

    private static Dictionary<string, (string Ret, string[] Args)> Prototypes()
    {
        string header = File.ReadAllText(TestSupport.RequireRepositoryFile(HeaderPath));
        var prototypes = new Dictionary<string, (string Ret, string[] Args)>(StringComparer.Ordinal);
        foreach (Match match in PrototypePattern().Matches(header))
        {
            string[] arguments = match.Groups["args"].Value.Split(',')
                .Select(argument => TypeOfParameter(argument.Trim()))
                .Where(type => type.Length > 0 && type != "void")
                .ToArray();
            prototypes.Add(match.Groups["name"].Value, (Squeeze(match.Groups["ret"].Value), arguments));
        }

        Assert.IsTrue(prototypes.Count > 0, "no prototypes found in nurt.h");
        return prototypes;
    }

    private static Dictionary<string, (string Name, string Ret, string[] Args)> Imports(string[] path)
    {
        string source = File.ReadAllText(TestSupport.RequireRepositoryFile(path));
        var imports = new Dictionary<string, (string Name, string Ret, string[] Args)>(StringComparer.Ordinal);
        foreach (Match match in ImportPattern().Matches(source))
        {
            string[] arguments = match.Groups["args"].Value.Split(',', StringSplitOptions.RemoveEmptyEntries)
                .Select(argument => argument.Trim())
                .Select(argument => argument[..argument.LastIndexOf(' ')])
                .ToArray();
            imports.Add(match.Groups["entry"].Value, (match.Groups["name"].Value, match.Groups["ret"].Value, arguments));
        }

        return imports;
    }

    /// <summary>Type of a parameter of a prototype, without the name and without blanks.</summary>
    private static string TypeOfParameter(string parameter)
    {
        Match match = ParameterPattern().Match(parameter);
        return match.Success && match.Groups["type"].Value.Trim().Length > 0
            ? Squeeze(match.Groups["type"].Value)
            : Squeeze(parameter);
    }

    private static string Squeeze(string text) => BlankPattern().Replace(text, string.Empty);

    /// <summary>Whether the C type <paramref name="cType"/> (without blanks) is passed as <paramref name="clr"/>.</summary>
    private static bool Accepts(string cType, Type clr) => cType switch
    {
        "nurt_status" => clr == typeof(CallStatus),
        "void" => clr == typeof(void),
        "uint32_t" => clr == typeof(uint),
        "nurt_variant" => clr == typeof(int),
        "constchar*" => clr == typeof(byte).MakePointerType(),
        "nurt_buf*" => clr == typeof(NativeBuffer).MakePointerType(),
        "nurt_engine*" => clr == typeof(EngineHandle) || clr == typeof(IntPtr),
        "nurt_engine**" => clr == typeof(EngineHandle).MakeByRefType(),
        "nurt_exec*" => clr == typeof(ExecHandle) || clr == typeof(IntPtr),
        "nurt_exec**" => clr == typeof(ExecHandle).MakeByRefType(),
        "nurt_event_cb" or "void*" => clr == typeof(IntPtr),
        _ => false,
    };

    private static string ReadString(byte[] bytes, int length)
    {
        fixed (byte* pointer = bytes)
        {
            var buffer = new NativeBuffer { Pointer = pointer, Length = (nuint)length, Capacity = (nuint)bytes.Length };
            return buffer.ReadString();
        }
    }

    private static JsonElement? ReadJson(string text)
    {
        byte[] bytes = Utf8.ToNative(text, "text")!;
        fixed (byte* pointer = bytes)
        {
            var buffer = new NativeBuffer { Pointer = pointer, Length = (nuint)(bytes.Length - 1), Capacity = (nuint)bytes.Length };
            return buffer.ReadJson("test");
        }
    }

    private static string FromTerminated(byte[] bytes)
    {
        fixed (byte* pointer = bytes)
        {
            return Utf8.FromNative(pointer);
        }
    }

    /// <summary>Calls the receiver of events as the library does, with the text terminated by a zero byte.</summary>
    private static void Deliver(EventQueue queue, byte[] text)
    {
        EventDelegate callback = Marshal.GetDelegateForFunctionPointer<EventDelegate>(EventCallback.Pointer);
        GCHandle state = GCHandle.Alloc(queue);
        IntPtr memory = Marshal.AllocHGlobal(text.Length + 1);
        try
        {
            Marshal.Copy(text, 0, memory, text.Length);
            Marshal.WriteByte(memory, text.Length, 0);
            callback(GCHandle.ToIntPtr(state), memory);
        }
        finally
        {
            Marshal.FreeHGlobal(memory);
            state.Free();
        }
    }

    private static async Task<List<NurtExecutionEvent>> ReadAsync(EventQueue queue)
    {
        queue.Complete();
        var events = new List<NurtExecutionEvent>();
        await foreach (NurtExecutionEvent item in queue.ReadAllAsync(CancellationToken.None))
        {
            events.Add(item);
        }

        return events;
    }
}
