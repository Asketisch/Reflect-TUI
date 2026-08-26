#[derive(Debug, Clone, Default)]
pub struct Arg0DispatchPaths;
pub fn arg0_dispatch_or_else<P>(_paths: Arg0DispatchPaths, _fallback: P) -> String
where
    P: FnOnce() -> String,
{
    _fallback()
}
