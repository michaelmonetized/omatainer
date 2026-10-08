use super::{Arc, LibItem, Weak};
use crate::library::{Catalog, search::{Indexed, Query, Row}};

const MAX_BYTES: usize=128*1024*1024;
const MAX_DELTA: usize=256;

pub(super) struct Index {
    records:Vec<Arc<Indexed>>,
    previous_rows:Weak<Vec<LibItem>>,
    previous_catalog:Weak<Catalog>,
    delta:Option<Vec<usize>>,
    bytes:usize,
    #[cfg(test)]
    prepared:usize,
}
/// Borrow the effective searchable fields of one exact current library row.
/// Takes its item, publication and neutral annotations; returns fields without file reads or alias substitution.
fn row<'a>(item:&'a LibItem,catalog:&'a Catalog,empty:&'a crate::library::annotations::Annotations)->Row<'a> {
    let track=catalog.track(&item.source);
    Row {title:&item.title,artist:&item.artist,key:&item.key,bpm:item.bpm.value(),seconds:item.length,played:item.last_play.is_some(),annotations:track.map_or(empty,|track|&track.annotations)}
}
impl Index {
    /// Prepare normalized search metadata on the existing library worker.
    /// Takes exact current and optional prior row/catalog publications; reuses unchanged records and refuses an index beyond its fixed retained byte limit.
    pub(super) fn build(rows:&Arc<Vec<LibItem>>,catalog:&Arc<Catalog>,previous:Option<(&Self,&Arc<Vec<LibItem>>,&Arc<Catalog>)>)->Result<Self,String> {
        let empty=crate::library::annotations::Annotations::default();
        let old_positions=previous.map(|(_,rows,_)|rows.iter().enumerate().map(|(index,item)|(&item.source,index)).collect::<std::collections::HashMap<_,_>>());
        let same_positions=previous.is_some_and(|(_,old,_)|rows.len()==old.len()&&rows.iter().zip(old.iter()).all(|(a,b)|a.source==b.source&&a.fingerprint==b.fingerprint));
        let mut next=Self {records:Vec::with_capacity(rows.len()),previous_rows:previous.map_or_else(Weak::new,|(_,rows,_)|Arc::downgrade(rows)),previous_catalog:previous.map_or_else(Weak::new,|(_,_,catalog)|Arc::downgrade(catalog)),delta:same_positions.then(Vec::new),bytes:rows.len()*std::mem::size_of::<Arc<Indexed>>(),#[cfg(test)] prepared:0};
        for (index,item) in rows.iter().enumerate() {
            let play_changed=same_positions&&previous.is_some_and(|(_,old,_)|item.last_play!=old[index].last_play);
            if play_changed {next.delta.as_mut().unwrap().push(index);}
            let fields=row(item,catalog,&empty);
            let version=catalog.version(&item.source,item.fingerprint);
            let key=crate::musical_key::effective(version,&item.key,catalog.track(&item.source).is_some_and(|track|track.locks.metadata)).0;
            let retained=previous.and_then(|(prior,old_rows,old_catalog)|{
                let old_index=*old_positions.as_ref()?.get(&item.source)?;
                let old=&old_rows[old_index];if item.fingerprint!=old.fingerprint {return None;} let old_fields=row(old,old_catalog,&empty);
                let old_key=crate::musical_key::effective(old_catalog.version(&old.source,old.fingerprint),&old.key,old_catalog.track(&old.source).is_some_and(|track|track.locks.metadata)).0;
                (item.title==old.title&&item.artist==old.artist&&key==old_key&&item.bpm==old.bpm&&item.length==old.length&&fields.annotations==old_fields.annotations).then(||prior.records[old_index].clone())
            });
            let record=retained.unwrap_or_else(||{
                if !play_changed {if let Some(delta)=&mut next.delta {delta.push(index);}}
                #[cfg(test)] {next.prepared+=1;}
                Arc::new(Indexed::new(Row {key:&key,..fields}))
            });
            next.bytes=next.bytes.saturating_add(record.bytes()+2*std::mem::size_of::<usize>());
            if next.bytes>MAX_BYTES {return Err("Library search index exceeds 128 MiB; shorten oversized metadata before indexed searching".into());}
            next.records.push(record);
        }
        next.bytes=next.bytes.saturating_add(std::mem::size_of::<Self>()+next.delta.as_ref().map_or(0,|delta|delta.capacity()*std::mem::size_of::<usize>()));
        if next.bytes>MAX_BYTES {return Err("Library search index exceeds 128 MiB; shorten oversized metadata before indexed searching".into());}
        if next.delta.as_ref().is_some_and(|delta|delta.len()>MAX_DELTA) {next.delta=None;}
        Ok(next)
    }
    /// Match a compiled query without Unicode allocation on the GUI thread.
    /// Takes an exact row position and current confirmed play state; returns the same field semantics as ordinary search.
    pub(super) fn matches(&self,query:&Query,index:usize,played:bool)->bool {query.matches_indexed(&self.records[index],played)}
    /// Apply only small metadata changes to an unchanged whole-library query.
    /// Takes the previous exact view publications; returns changed row positions only when identity and order are unchanged.
    pub(super) fn delta_for(&self,rows:&Weak<Vec<LibItem>>,catalog:&Weak<Catalog>)->Option<&[usize]> {
        (Weak::ptr_eq(rows,&self.previous_rows)&&Weak::ptr_eq(catalog,&self.previous_catalog)).then(||self.delta.as_deref()).flatten()
    }
    /// Account for the additional retained search metadata.
    /// Takes no arguments; returns record storage and shared-owner overhead in bytes.
    pub(super) fn bytes(&self)->usize {self.bytes}
}

#[cfg(test)]
mod tests;

impl Index {
    /// Compare full prepared metadata while preserving unavailable values last.
    /// Takes two exact row positions, current play history and chosen columns; returns their deterministic value order without normalized-text allocation.
    pub(super) fn compare(&self,a:usize,b:usize,rows:&[LibItem],history:&crate::ui::play_history::History,sorts:[Option<crate::preferences::library_layout::Sort>;2])->std::cmp::Ordering {
        use crate::preferences::library_layout::Column;
        use crate::ui::library_layout::sort::optional;
        use std::cmp::Ordering;
        let left=self.records[a].row(false);let right=self.records[b].row(false);
        for sort in sorts.into_iter().flatten() {
            let text=match sort.column {
                Column::Title=>Some((left.title,right.title)),Column::Artist=>Some((left.artist,right.artist)),Column::Key=>Some((left.key,right.key)),
                Column::Group=>Some((left.annotations.group.as_str(),right.annotations.group.as_str())),Column::Notes=>Some((left.annotations.notes.as_str(),right.annotations.notes.as_str())),
                Column::Tags=>Some((self.records[a].sorted_tags(),self.records[b].sorted_tags())),_=>None,
            };
            let order=if let Some((a,b))=text {let order=a.cmp(b);if sort.descending {order.reverse()} else {order}}
            else {match sort.column {
                Column::Bpm=>optional(&left.bpm.filter(|v|v.is_finite()).map(f64::from),&right.bpm.filter(|v|v.is_finite()).map(f64::from),sort.descending,|a,b|a.total_cmp(b)),
                Column::Length=>optional(&left.seconds.filter(|v|v.is_finite()),&right.seconds.filter(|v|v.is_finite()),sort.descending,|a,b|a.total_cmp(b)),
                Column::Rating=>optional(&Some(left.annotations.rating),&Some(right.annotations.rating),sort.descending,Ord::cmp),
                Column::Played=>optional(&history.get(&rows[a]).or(rows[a].last_play),&history.get(&rows[b]).or(rows[b].last_play),sort.descending,Ord::cmp),
                Column::Color=>optional(&left.annotations.color,&right.annotations.color,sort.descending,Ord::cmp),_=>unreachable!(),
            }};
            if order!=Ordering::Equal {return order;}
        }
        Ordering::Equal
    }
}
