using Microsoft.VisualStudio.TestTools.UnitTesting;

// Tests start child processes and read process-wide state (the loaded library and the
// count of live execution handles), so they run one at a time.
[assembly: DoNotParallelize]
