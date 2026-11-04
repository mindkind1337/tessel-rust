# Tessel process ownership patch

Based on crates.io portable-pty 0.9.0, MIT, Wez Furlong. Original license is
preserved in LICENSE.md. No application or provider code is bundled here.

On Windows, ConPTY creates its child suspended, assigns it to a dedicated
KILL_ON_JOB_CLOSE job with no breakaway permission, then resumes its first thread.
Initialization errors terminate the suspended child. Child::kill and its cloned
killer terminate the entire owned job and propagate errors. The new
process_tree_exited method queries the job's active process count; unsupported
backends return an error rather than certifying descendants they cannot observe.

This avoids the descendant race in assigning a job to an already-running process.
Tessel uses the accounting result before starting a replacement terminal. Keep
this patch explicit when upgrading portable-pty; do not silently revert to killing
only the direct shell wrapper.
