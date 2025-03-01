use std::borrow::Cow;

use rand::{rngs::StdRng, SeedableRng};

use crate::Machine;

use super::{
    bootstrapping_compile, current_dir, import_builtin_impls, libraries, load_module, Atom,
    CompilationTarget, IndexStore, ListingSource, MachineArgs, MachineState, Stream, StreamOptions,
};

/// Describes how the streams of a [`Machine`](crate::Machine) will be handled.
#[derive(Default)]
pub struct StreamConfig {
    inner: StreamConfigInner,
}

impl StreamConfig {
    /// Binds the input, output and error streams to stdin, stdout and stderr.
    pub fn stdio() -> Self {
        StreamConfig {
            inner: StreamConfigInner::Stdio,
        }
    }

    /// Binds the output stream to a memory buffer, and the error stream to stderr.
    ///
    /// The input stream is ignored.
    pub fn in_memory() -> Self {
        StreamConfig {
            inner: StreamConfigInner::Memory,
        }
    }
}

#[derive(Default)]
enum StreamConfigInner {
    Stdio,
    #[default]
    Memory,
}

/// Describes how a [`Machine`](crate::Machine) will be configured.
pub struct MachineBuilder {
    pub(crate) streams: StreamConfig,
    pub(crate) toplevel: Cow<'static, str>,
}

impl Default for MachineBuilder {
    /// Defaults to using in-memory streams.
    fn default() -> Self {
        MachineBuilder {
            streams: Default::default(),
            toplevel: default_toplevel().into(),
        }
    }
}

impl MachineBuilder {
    /// Creates a default configuration.
    pub fn new() -> Self {
        Default::default()
    }

    /// Uses the given `crate::StreamConfig` in this configuration.
    pub fn with_streams(mut self, streams: StreamConfig) -> Self {
        self.streams = streams;
        self
    }

    /// Uses the given toplevel in this configuration.
    pub fn with_toplevel(mut self, toplevel: impl Into<Cow<'static, str>>) -> Self {
        self.toplevel = toplevel.into();
        self
    }

    /// Builds the [`Machine`](crate::Machine) from this configuration.
    pub fn build(self) -> Machine {
        let args = MachineArgs::new();
        let mut machine_st = MachineState::new();

        let (user_input, user_output, user_error) = match self.streams.inner {
            StreamConfigInner::Stdio => (
                Stream::stdin(&mut machine_st.arena, args.add_history),
                Stream::stdout(&mut machine_st.arena),
                Stream::stderr(&mut machine_st.arena),
            ),
            StreamConfigInner::Memory => (
                Stream::Null(StreamOptions::default()),
                Stream::from_owned_string("".to_owned(), &mut machine_st.arena),
                Stream::stderr(&mut machine_st.arena),
            ),
        };

        let mut wam = Machine {
            machine_st,
            indices: IndexStore::new(),
            code: vec![],
            user_input,
            user_output,
            user_error,
            load_contexts: vec![],
            #[cfg(feature = "ffi")]
            foreign_function_table: Default::default(),
            rng: StdRng::from_entropy(),
        };

        let mut lib_path = current_dir();

        lib_path.pop();
        lib_path.push("lib");

        wam.add_impls_to_indices();

        bootstrapping_compile(
            Stream::from_static_string(
                libraries::get("ops_and_meta_predicates")
                    .expect("library ops_and_meta_predicates should exist"),
                &mut wam.machine_st.arena,
            ),
            &mut wam,
            ListingSource::from_file_and_path(
                atom!("ops_and_meta_predicates.pl"),
                lib_path.clone(),
            ),
        )
        .unwrap();

        bootstrapping_compile(
            Stream::from_static_string(
                libraries::get("builtins").expect("library builtins should exist"),
                &mut wam.machine_st.arena,
            ),
            &mut wam,
            ListingSource::from_file_and_path(atom!("builtins.pl"), lib_path.clone()),
        )
        .unwrap();

        if let Some(builtins) = wam.indices.modules.get_mut(&atom!("builtins")) {
            load_module(
                &mut wam.machine_st,
                &mut wam.indices.code_dir,
                &mut wam.indices.op_dir,
                &mut wam.indices.meta_predicates,
                &CompilationTarget::User,
                builtins,
            );

            import_builtin_impls(&wam.indices.code_dir, builtins);
        } else {
            unreachable!()
        }

        lib_path.pop(); // remove the "lib" at the end

        bootstrapping_compile(
            Stream::from_static_string(include_str!("../loader.pl"), &mut wam.machine_st.arena),
            &mut wam,
            ListingSource::from_file_and_path(atom!("loader.pl"), lib_path.clone()),
        )
        .unwrap();

        wam.configure_modules();

        if let Some(loader) = wam.indices.modules.get(&atom!("loader")) {
            load_module(
                &mut wam.machine_st,
                &mut wam.indices.code_dir,
                &mut wam.indices.op_dir,
                &mut wam.indices.meta_predicates,
                &CompilationTarget::User,
                loader,
            );
        } else {
            unreachable!()
        }

        wam.load_special_forms();
        wam.load_top_level(self.toplevel);
        wam.configure_streams();

        wam
    }
}

/// Returns a static string slice to the default toplevel
pub fn default_toplevel() -> &'static str {
    include_str!("../toplevel.pl")
}
