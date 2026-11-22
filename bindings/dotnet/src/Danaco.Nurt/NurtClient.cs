// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace Danaco.Nurt;

/// <summary>
/// Client of Danaco Nurt. Runs programs and reads the catalog, help and journal through
/// the library <c>libnurt</c> or a child process <c>nurt serve --stdio</c>.
/// </summary>
/// <remarks>
/// <para>
/// A client is safe for use from several threads; calls may run in parallel. Errors of the
/// language are returned in <see cref="NurtResponse.Error"/>; failures of the call raise
/// <see cref="NurtCallException"/>.
/// </para>
/// <para>
/// With the process backend a child process that exits is started again by the next call;
/// calls in progress at that moment fail with <see cref="CallStatus.Internal"/>.
/// </para>
/// </remarks>
/// <example>
/// <code>
/// await using var nurt = new NurtClient();
/// NurtResponse response = await nurt.RunAsync("find \"/home/user/project\" glob: \"**/*.md\"");
/// Console.WriteLine(response.Ok ? response.Text : response.Error!.Message);
/// </code>
/// </example>
public sealed class NurtClient : IDisposable, IAsyncDisposable
{
    private readonly IBackend _backend;
    private readonly NurtClientOptions _options;
    private int _disposed;

    /// <summary>Creates a client with the default options.</summary>
    /// <exception cref="NurtCallException">No usable library or executable was found, or the library failed to open.</exception>
    public NurtClient()
        : this(new NurtClientOptions())
    {
    }

    /// <summary>Creates a client.</summary>
    /// <param name="options">Settings of the client.</param>
    /// <exception cref="ArgumentNullException"><paramref name="options"/> is <see langword="null"/>.</exception>
    /// <exception cref="NurtCallException">
    /// The requested artifact was not found (<see cref="CallStatus.Io"/>), the library implements an
    /// incompatible interface version (<see cref="CallStatus.AbiMismatch"/>) or the library failed to open.
    /// </exception>
    public NurtClient(NurtClientOptions options)
        : this(options, new ArtifactLocator())
    {
    }

    internal NurtClient(NurtClientOptions options, ArtifactLocator locator)
    {
        Guard.NotNull(options, nameof(options));
        _options = options;
        _backend = BackendFactory.Create(options, locator);
    }

    /// <summary>Backend in use: <see cref="BackendKind.Library"/> or <see cref="BackendKind.Process"/>.</summary>
    public BackendKind Backend => _backend.Kind;

    /// <summary>Runs a program text and waits for the response.</summary>
    /// <param name="program">Program text in any notation variant.</param>
    /// <param name="options">Call options; <see langword="null"/> for the defaults.</param>
    /// <param name="cancellationToken">Cancels the execution.</param>
    /// <returns>The response; an error of the language is in <see cref="NurtResponse.Error"/>.</returns>
    /// <exception cref="NurtCallException">The call failed.</exception>
    /// <exception cref="NurtLanguageException">The response carries an error and <see cref="RunOptions.ThrowOnLanguageError"/> is set.</exception>
    /// <exception cref="OperationCanceledException"><paramref name="cancellationToken"/> was cancelled; cancellation was requested from Danaco Nurt.</exception>
    public Task<NurtResponse> RunAsync(string program, RunOptions? options = null, CancellationToken cancellationToken = default) =>
        RunAsync(Request.FromProgram(program), options, cancellationToken);

    /// <summary>Runs a request and waits for the response.</summary>
    /// <param name="request">Program text, syntax tree, cursor or accepted candidate.</param>
    /// <param name="options">Call options; <see langword="null"/> for the defaults.</param>
    /// <param name="cancellationToken">Cancels the execution.</param>
    /// <returns>The response; an error of the language is in <see cref="NurtResponse.Error"/>.</returns>
    /// <exception cref="ArgumentNullException"><paramref name="request"/> is <see langword="null"/>.</exception>
    /// <exception cref="NurtCallException">The call failed.</exception>
    /// <exception cref="NurtLanguageException">The response carries an error and <see cref="RunOptions.ThrowOnLanguageError"/> is set.</exception>
    /// <exception cref="OperationCanceledException"><paramref name="cancellationToken"/> was cancelled; cancellation was requested from Danaco Nurt.</exception>
    public async Task<NurtResponse> RunAsync(Request request, RunOptions? options = null, CancellationToken cancellationToken = default)
    {
        Guard.NotNull(request, nameof(request));
        IBackend backend = ActiveBackend();
        NurtResponse response = await backend.RunAsync(WireFormat.Envelope(request, options, _options), cancellationToken).ConfigureAwait(false);
        return response.ThrowIfRequested(options);
    }

    /// <summary>Starts a program text and returns without waiting for the response.</summary>
    /// <param name="program">Program text in any notation variant.</param>
    /// <param name="options">Call options; <see langword="null"/> for the defaults.</param>
    /// <param name="cancellationToken">Cancels starting the execution.</param>
    /// <returns>The running execution; dispose it after use.</returns>
    /// <exception cref="NurtCallException">The execution could not be started.</exception>
    public Task<Execution> StartAsync(string program, RunOptions? options = null, CancellationToken cancellationToken = default) =>
        StartAsync(Request.FromProgram(program), options, cancellationToken);

    /// <summary>Starts a request and returns without waiting for the response.</summary>
    /// <param name="request">Program text, syntax tree, cursor or accepted candidate.</param>
    /// <param name="options">Call options; <see langword="null"/> for the defaults.</param>
    /// <param name="cancellationToken">Cancels starting the execution.</param>
    /// <returns>The running execution; dispose it after use.</returns>
    /// <exception cref="ArgumentNullException"><paramref name="request"/> is <see langword="null"/>.</exception>
    /// <exception cref="NurtCallException">The execution could not be started.</exception>
    public async Task<Execution> StartAsync(Request request, RunOptions? options = null, CancellationToken cancellationToken = default)
    {
        Guard.NotNull(request, nameof(request));
        IBackend backend = ActiveBackend();
        var events = new EventQueue();
        IExecutionHandle handle = await backend.StartAsync(WireFormat.Envelope(request, options, _options), events, cancellationToken).ConfigureAwait(false);
        return new Execution(handle, events, options);
    }

    /// <summary>Binds a request and classifies its effect without running it.</summary>
    /// <param name="request">Program text, syntax tree or accepted candidate.</param>
    /// <param name="options">Call options; <see cref="RunOptions.Preview"/> has no further effect.</param>
    /// <param name="cancellationToken">Cancels waiting for the result.</param>
    /// <returns>Bound tree, targets, paths, external programs, network addresses and notable effects.</returns>
    /// <exception cref="ArgumentNullException"><paramref name="request"/> is <see langword="null"/>.</exception>
    /// <exception cref="NurtCallException">The call failed.</exception>
    public Task<CheckResult> CheckAsync(Request request, RunOptions? options = null, CancellationToken cancellationToken = default)
    {
        Guard.NotNull(request, nameof(request));
        return ActiveBackend().CheckAsync(WireFormat.Envelope(request, options, _options), cancellationToken);
    }

    /// <summary>Parses a program text into a syntax tree <c>nurt-ast/1</c>. No effect on the world.</summary>
    /// <param name="program">Program text.</param>
    /// <param name="variant">Notation variant of the text.</param>
    /// <param name="cancellationToken">Cancels waiting for the result.</param>
    /// <returns>The syntax tree and diagnostics.</returns>
    /// <exception cref="ArgumentNullException"><paramref name="program"/> is <see langword="null"/>.</exception>
    /// <exception cref="NurtCallException">The call failed.</exception>
    public Task<ParseResult> ParseAsync(string program, NotationVariant variant = NotationVariant.A, CancellationToken cancellationToken = default)
    {
        Guard.NotNull(program, nameof(program));
        return ActiveBackend().ParseAsync(program, variant, cancellationToken);
    }

    /// <summary>Prints a syntax tree in the canonical text of a notation variant. No effect on the world.</summary>
    /// <param name="ast">Syntax tree <c>nurt-ast/1</c>.</param>
    /// <param name="variant">Notation variant of the text.</param>
    /// <param name="cancellationToken">Cancels waiting for the result.</param>
    /// <returns>The program text.</returns>
    /// <exception cref="NurtCallException">The call failed, for example the tree is not valid (<see cref="CallStatus.InvalidArgument"/>).</exception>
    public Task<string> PrintAsync(JsonElement ast, NotationVariant variant = NotationVariant.A, CancellationToken cancellationToken = default) =>
        ActiveBackend().PrintAsync(ast, variant, cancellationToken);

    /// <summary>Corrects the written form of a program without running it.</summary>
    /// <param name="program">Program text.</param>
    /// <param name="variant">Notation variant of the text.</param>
    /// <param name="cancellationToken">Cancels waiting for the result.</param>
    /// <returns>The corrected program and the list of corrections.</returns>
    /// <exception cref="ArgumentNullException"><paramref name="program"/> is <see langword="null"/>.</exception>
    /// <exception cref="NurtCallException">The call failed.</exception>
    public Task<CorrectResult> CorrectAsync(string program, NotationVariant variant = NotationVariant.A, CancellationToken cancellationToken = default)
    {
        Guard.NotNull(program, nameof(program));
        return ActiveBackend().CorrectAsync(program, variant, cancellationToken);
    }

    /// <summary>Returns the next page of a paged result.</summary>
    /// <param name="cursor">Cursor from <see cref="NurtResponse.NextCursor"/>.</param>
    /// <param name="limit">Records per page; <see langword="null"/> for the limit of the original request.</param>
    /// <param name="cancellationToken">Cancels waiting for the result.</param>
    /// <returns>The page; an expired cursor is an error of the language.</returns>
    /// <exception cref="ArgumentNullException"><paramref name="cursor"/> is <see langword="null"/>.</exception>
    /// <exception cref="ArgumentOutOfRangeException"><paramref name="limit"/> is less than 1.</exception>
    /// <exception cref="NurtCallException">The call failed.</exception>
    public Task<NurtResponse> PageAsync(string cursor, int? limit = null, CancellationToken cancellationToken = default)
    {
        Guard.NotNull(cursor, nameof(cursor));
        if (limit < 1)
        {
            throw new ArgumentOutOfRangeException(nameof(limit), limit, "limit must be at least 1");
        }

        return ActiveBackend().PageAsync(cursor, limit, cancellationToken);
    }

    /// <summary>Returns the description of the language for tool integrations, as text.</summary>
    /// <param name="layer">Layer of the description, for example <c>core</c> or <c>group</c>.</param>
    /// <param name="name">Name of the group or command; <see langword="null"/> for the whole layer.</param>
    /// <param name="language">Language of the text; <see langword="null"/> for <see cref="NurtClientOptions.Language"/>.</param>
    /// <param name="cancellationToken">Cancels waiting for the result.</param>
    /// <returns>The description.</returns>
    /// <exception cref="ArgumentNullException"><paramref name="layer"/> is <see langword="null"/>.</exception>
    /// <exception cref="NurtCallException">The call failed.</exception>
    public Task<string> DescribeAsync(string layer = "core", string? name = null, string? language = null, CancellationToken cancellationToken = default)
    {
        Guard.NotNull(layer, nameof(layer));
        var selector = new DescribeParams { Layer = layer, Name = name, Language = language ?? _options.Language };
        return ActiveBackend().DescribeAsync(selector, cancellationToken);
    }

    /// <summary>Returns built-in help on a topic, as text.</summary>
    /// <param name="topic">Topic; <see langword="null"/> for the index of topics.</param>
    /// <param name="cancellationToken">Cancels waiting for the result.</param>
    /// <returns>The help text.</returns>
    /// <exception cref="NurtCallException">The call failed.</exception>
    public Task<string> HelpAsync(string? topic = null, CancellationToken cancellationToken = default) =>
        ActiveBackend().HelpAsync(topic, cancellationToken);

    /// <summary>Returns the command catalog as JSON.</summary>
    /// <param name="what">Object selecting a group or a command; <see langword="null"/> for the whole catalog.</param>
    /// <param name="cancellationToken">Cancels waiting for the result.</param>
    /// <returns>The catalog <c>nurt-catalog/1</c>.</returns>
    /// <exception cref="NurtCallException">The call failed.</exception>
    public Task<JsonElement> CatalogAsync(JsonElement? what = null, CancellationToken cancellationToken = default) =>
        ActiveBackend().CatalogAsync(what, cancellationToken);

    /// <summary>Returns a published JSON schema by name.</summary>
    /// <param name="which">Name of the schema: <c>ast</c>, <c>bound</c>, <c>report</c>, <c>request</c>, <c>config</c>, <c>catalog</c>, <c>value</c> or <c>tool</c>.</param>
    /// <param name="cancellationToken">Cancels waiting for the result.</param>
    /// <returns>The schema.</returns>
    /// <exception cref="ArgumentNullException"><paramref name="which"/> is <see langword="null"/>.</exception>
    /// <exception cref="NurtCallException">The name is unknown (<see cref="CallStatus.InvalidArgument"/>) or the call failed.</exception>
    public Task<JsonElement> SchemaAsync(string which, CancellationToken cancellationToken = default)
    {
        Guard.NotNull(which, nameof(which));
        return ActiveBackend().SchemaAsync(which, cancellationToken);
    }

    /// <summary>Returns the journal history as JSON.</summary>
    /// <param name="query">Object with the parameters of the command <c>history</c>; <see langword="null"/> for the defaults.</param>
    /// <param name="cancellationToken">Cancels waiting for the result.</param>
    /// <returns>The history.</returns>
    /// <exception cref="NurtCallException">The call failed.</exception>
    public Task<JsonElement> HistoryAsync(JsonElement? query = null, CancellationToken cancellationToken = default) =>
        ActiveBackend().HistoryAsync(query, cancellationToken);

    /// <summary>Returns free space and journal usage of a volume as JSON.</summary>
    /// <param name="volume">A path on the volume; <see langword="null"/> for all volumes.</param>
    /// <param name="cancellationToken">Cancels waiting for the result.</param>
    /// <returns>The state of the volumes.</returns>
    /// <exception cref="NurtCallException">The call failed.</exception>
    public Task<JsonElement> SpaceAsync(string? volume = null, CancellationToken cancellationToken = default) =>
        ActiveBackend().SpaceAsync(volume, cancellationToken);

    /// <summary>
    /// Returns information about Danaco Nurt as JSON: version, interface version, language and
    /// format versions, catalog revision, features, target and build.
    /// </summary>
    /// <param name="cancellationToken">Cancels waiting for the result.</param>
    /// <returns>The information.</returns>
    /// <exception cref="NurtCallException">The call failed.</exception>
    public Task<JsonElement> InfoAsync(CancellationToken cancellationToken = default) =>
        ActiveBackend().InfoAsync(cancellationToken);

    /// <summary>
    /// Closes the client. The library engine is closed after its running executions are
    /// cancelled; the child process receives the end of its input and is terminated when it
    /// does not exit within <see cref="NurtClientOptions.ShutdownTimeout"/>.
    /// </summary>
    public void Dispose()
    {
        if (Interlocked.Exchange(ref _disposed, 1) == 0)
        {
            _backend.Dispose();
        }

        GC.SuppressFinalize(this);
    }

    /// <summary>Closes the client; see <see cref="Dispose"/>.</summary>
    /// <returns>A task that completes when the backend is closed.</returns>
    public async ValueTask DisposeAsync()
    {
        if (Interlocked.Exchange(ref _disposed, 1) == 0)
        {
            await _backend.DisposeAsync().ConfigureAwait(false);
        }

        GC.SuppressFinalize(this);
    }

    private IBackend ActiveBackend()
    {
        Guard.NotDisposed(Volatile.Read(ref _disposed) != 0, this);
        return _backend;
    }
}
