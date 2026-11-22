using System;
using System.Text;

namespace Danaco.Nurt;

internal static unsafe class Utf8
{
    private static readonly UTF8Encoding Strict = new(encoderShouldEmitUTF8Identifier: false, throwOnInvalidBytes: true);

    /// <summary>Encodes a string as UTF-8 followed by a terminating zero byte.</summary>
    public static byte[]? ToNative(string? value, string parameterName)
    {
        if (value is null)
        {
            return null;
        }

        if (ContainsZero(value))
        {
            throw new ArgumentException("text must not contain the character U+0000", parameterName);
        }

        byte[] result = new byte[Strict.GetByteCount(value) + 1];
        Strict.GetBytes(value, 0, value.Length, result, 0);
        return result;
    }

    /// <summary>Appends a terminating zero byte to UTF-8 text.</summary>
    public static byte[] Terminate(byte[] text)
    {
        byte[] result = new byte[text.Length + 1];
        Buffer.BlockCopy(text, 0, result, 0, text.Length);
        return result;
    }

    /// <summary>Decodes a zero-terminated UTF-8 string owned by the library.</summary>
    public static string FromNative(byte* text)
    {
        if (text is null)
        {
            return string.Empty;
        }

        int length = 0;
        while (text[length] != 0)
        {
            length++;
        }

        return Strict.GetString(text, length);
    }

    /// <summary>Decodes UTF-8 text of a known length owned by the library.</summary>
    public static string FromNative(byte* text, nuint length) =>
        length == 0 ? string.Empty : Strict.GetString(text, checked((int)length));

    private static bool ContainsZero(string value)
    {
#if NET8_0_OR_GREATER
        return value.Contains('\0', StringComparison.Ordinal);
#else
        return value.IndexOf('\0') >= 0;
#endif
    }
}
