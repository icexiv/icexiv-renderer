pub use icexiv_data::havok::{Error, Result};
pub(super) fn spend(value: &mut usize, n: usize, name: &'static str) -> Result<()> {
    *value = value.checked_sub(n).ok_or(Error::BudgetExceeded(name))?;
    Ok(())
}
pub(super) fn reserve<T>(n: usize, bytes: &mut usize) -> Result<Vec<T>> {
    spend(
        bytes,
        n.checked_mul(std::mem::size_of::<T>())
            .ok_or("allocation size overflow")?,
        "allocation",
    )?;
    let mut v = Vec::new();
    v.try_reserve_exact(n)
        .map_err(|_| Error::AllocationFailed)?;
    Ok(v)
}
