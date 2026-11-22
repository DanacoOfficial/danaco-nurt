// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System;
using System.Globalization;
using System.IO;
using System.Reflection;
using System.Text.Json;

namespace Danaco.Nurt;

internal static class WireFormat
{
    public const string EnvelopeVersion = "1.0";

    public const string Protocol = "nurt-rpc/1";

    public const string ClientName = "Danaco.Nurt";

    public static readonly string ClientVersion =
        typeof(WireFormat).Assembly.GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion
        ?? "0.0.0";

    public static byte[] Envelope(Request request, RunOptions? options, NurtClientOptions client)
    {
        var envelope = new RequestEnvelope
        {
            Program = request.Program,
            Variant = request.Variant is { } variant ? VariantName(variant) : null,
            Ast = request.Ast,
            Cursor = request.Cursor,
            Accept = request.Accept,
            Options = Options(options, client),
        };
        return JsonSerializer.SerializeToUtf8Bytes(envelope, WireJsonContext.Default.RequestEnvelope);
    }

    /// <summary>
    /// Writes a JSON value without insignificant white space, so that it fits on one line of
    /// the protocol and is passed to the library unchanged in meaning.
    /// </summary>
    public static byte[] Compact(JsonElement value)
    {
        using var stream = new MemoryStream();
        using (var writer = new Utf8JsonWriter(stream))
        {
            value.WriteTo(writer);
        }

        return stream.ToArray();
    }

    public static string VariantName(NotationVariant variant) => variant switch
    {
        NotationVariant.A => "a",
        NotationVariant.B => "b",
        NotationVariant.C => "c",
        _ => throw new ArgumentOutOfRangeException(nameof(variant), variant, "notation variant must be A, B or C"),
    };

    public static int NativeVariant(NotationVariant variant) => variant switch
    {
        NotationVariant.A or NotationVariant.B or NotationVariant.C => (int)variant + 1,
        _ => throw new ArgumentOutOfRangeException(nameof(variant), variant, "notation variant must be A, B or C"),
    };

    public static string Duration(TimeSpan value)
    {
        if (value < TimeSpan.Zero)
        {
            throw new ArgumentOutOfRangeException(nameof(value), value, "timeout must not be negative");
        }

        return ((long)value.TotalMilliseconds).ToString(CultureInfo.InvariantCulture) + "ms";
    }

    private static EnvelopeOptions Options(RunOptions? options, NurtClientOptions client) => new()
    {
        Preview = options?.Preview,
        Base = options?.Base,
        Limit = options?.Limit,
        Fields = options?.Fields,
        Format = FormatName(options?.Format),
        Verbosity = VerbosityName(options?.Verbosity),
        Diff = DiffName(options?.Diff),
        DiffContext = options?.DiffContext,
        MaxChars = options?.MaxChars,
        SaveTo = options?.SaveTo,
        Timeout = options?.Timeout is { } timeout ? Duration(timeout) : null,
        Session = options?.Session ?? client.Session,
        Language = options?.Language ?? client.Language,
        Script = options?.Script,
    };

    private static string? FormatName(OutputFormat? format) => format switch
    {
        null => null,
        OutputFormat.Text => "text",
        OutputFormat.Json => "json",
        OutputFormat.Markdown => "markdown",
        OutputFormat.Compact => "compact",
        _ => throw new ArgumentOutOfRangeException(nameof(format), format, "unknown output format"),
    };

    private static string? VerbosityName(Verbosity? verbosity) => verbosity switch
    {
        null => null,
        Verbosity.Quiet => "quiet",
        Verbosity.Normal => "normal",
        Verbosity.Detailed => "detailed",
        _ => throw new ArgumentOutOfRangeException(nameof(verbosity), verbosity, "unknown verbosity"),
    };

    private static string? DiffName(DiffMode? diff) => diff switch
    {
        null => null,
        DiffMode.None => "none",
        DiffMode.Stat => "stat",
        DiffMode.Unified => "unified",
        _ => throw new ArgumentOutOfRangeException(nameof(diff), diff, "unknown diff mode"),
    };
}
