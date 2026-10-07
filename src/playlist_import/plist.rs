use super::*;
use quick_xml::{Reader, events::Event};
use std::collections::BTreeMap;

#[derive(Debug)]
enum Value { Text(String), Integer(String), Bool(bool), Array(Vec<Value>), Dict(BTreeMap<String,Value>), Other }
impl Value {
    fn dict(&self)->Result<&BTreeMap<String,Value>,String> {if let Self::Dict(v)=self {Ok(v)}else{Err("Expected an Apple plist dictionary".into())}}
    fn array(&self)->Result<&[Value],String> {if let Self::Array(v)=self {Ok(v)}else{Err("Expected an Apple plist array".into())}}
    fn text(&self)->Result<&str,String> {if let Self::Text(v)=self {Ok(v)}else{Err("Expected an Apple plist string".into())}}
    fn number(&self)->Result<u64,String> {if let Self::Integer(v)=self {v.parse().map_err(|_|"Track IDs must be nonnegative 64-bit integers".into())}else{Err("Expected an Apple plist integer".into())}}
}
struct Parser<'a,C:Fn()->bool> { reader:Reader<&'a [u8]>, check:&'a C, nodes:usize, text:usize }
impl<'a,C:Fn()->bool> Parser<'a,C> {
    fn event(&mut self)->Result<Event<'a>,String> {
        active(self.check)?;self.nodes+=1;
        if self.nodes>500000 {return Err("Apple XML exceeds 500000 parser events".into());}
        self.reader.read_event().map_err(|e|format!("Invalid Apple XML: {e}"))
    }
    fn significant(&mut self)->Result<Event<'a>,String> {
        loop {match self.event()? {Event::Comment(_)=>{},Event::Text(t) if t.decode().map_err(|e|e.to_string())?.trim().is_empty()=>{},event=>return Ok(event)}}
    }
    fn scalar(&mut self,tag:&[u8])->Result<String,String> {
        let mut text=String::new();
        loop {
            let part=match self.event()? {
                Event::Text(value)=>value.xml10_content().map_err(|e|e.to_string())?.into_owned(),
                Event::CData(value)=>value.decode().map_err(|e|e.to_string())?.into_owned(),
                Event::GeneralRef(value)=>{
                    if let Some(ch)=value.resolve_char_ref().map_err(|e|e.to_string())? {ch.to_string()}
                    else {match value.decode().map_err(|e|e.to_string())?.as_ref() {"amp"=>"&","lt"=>"<","gt"=>">","quot"=>"\"","apos"=>"'",_=>return Err("External or custom XML entities are unsupported".into())}.into()}
                },
                Event::End(end) if end.name().as_ref()==tag=>break,
                Event::Comment(_)=>continue,
                _=>return Err("Invalid nested content in an Apple plist scalar".into()),
            };
            if part.chars().any(|c|matches!(c as u32,0..=8|11..=12|14..=31|0xfffe|0xffff)) {return Err("Invalid XML control character".into());}
            self.text+=part.len();
            if self.text>16*1024*1024 || text.len()+part.len()>256*1024 {return Err("Apple XML exceeds its text budget".into());}
            text.push_str(&part);
        }
        Ok(text)
    }
    fn value(&mut self,event:Event<'a>,depth:usize)->Result<Value,String> {
        if depth>32 {return Err("Apple XML nesting exceeds 32 levels".into());}
        let Event::Start(start)=event else {return Err("Expected an Apple plist value".into())};
        if start.attributes().next().is_some() {return Err("Unexpected attributes on an Apple plist value".into());}
        let tag=start.name().as_ref().to_vec();
        match tag.as_slice() {
            b"dict"=>{
                let mut dict=BTreeMap::new();
                loop {let event=self.significant()?;if matches!(&event,Event::End(e) if e.name().as_ref()==b"dict") {break;}
                    let Event::Start(key)=event else {return Err("Apple dictionary needs key/value pairs".into())};
                    if key.name().as_ref()!=b"key" || key.attributes().next().is_some() {return Err("Apple dictionary key is invalid".into());}
                    let key=self.scalar(b"key")?;if key.len()>4096 {return Err("Apple dictionary key is oversized".into());}
                    let event=self.significant()?;let value=self.value(event,depth+1)?;
                    if dict.insert(key,value).is_some() {return Err("Duplicate Apple dictionary key; import refused".into());}
                }Ok(Value::Dict(dict))
            },
            b"array"=>{
                let mut values=Vec::new();loop {let event=self.significant()?;if matches!(&event,Event::End(e) if e.name().as_ref()==b"array") {break;}values.push(self.value(event,depth+1)?);}Ok(Value::Array(values))
            },
            b"string"=>Ok(Value::Text(self.scalar(&tag)?)),
            b"integer"=>{let text=self.scalar(&tag)?;text.trim().parse::<i128>().map_err(|_|"Invalid Apple plist integer")?;Ok(Value::Integer(text.trim().into()))},
            b"true"|b"false"=>{if !self.scalar(&tag)?.trim().is_empty() {return Err("Invalid Apple plist boolean".into());}Ok(Value::Bool(tag==b"true"))},
            b"data"|b"date"|b"real"=>{self.scalar(&tag)?;Ok(Value::Other)},
            _=>Err("Unsupported element in Apple XML plist".into()),
        }
    }
}
pub(super) fn utf8(bytes:&[u8])->Result<String,String> {
    if bytes.starts_with(&[0xff,0xfe]) || bytes.starts_with(&[0xfe,0xff]) {
        if (bytes.len()-2)%2!=0 {return Err("Truncated UTF-16 XML".into());}
        let little=bytes[0]==0xff;let words=bytes[2..].chunks_exact(2).map(|b|if little {u16::from_le_bytes([b[0],b[1]])}else{u16::from_be_bytes([b[0],b[1]])}).collect::<Vec<_>>();
        let mut text=String::from_utf16(&words).map_err(|_|"Invalid UTF-16 XML")?;
        if let Some(end)=text.find("?>") {let header=text[..end].replace("UTF-16LE","UTF-8").replace("UTF-16BE","UTF-8").replace("utf-16le","utf-8").replace("utf-16be","utf-8").replace("UTF-16","UTF-8").replace("utf-16","utf-8");text.replace_range(..end,&header);}
        Ok(text)
    }else{std::str::from_utf8(bytes).map(|s|s.trim_start_matches('\u{feff}').into()).map_err(|_|"Apple XML requires UTF-8 or BOM-labelled UTF-16".into())}
}
fn text<'a>(dict:&'a BTreeMap<String,Value>,key:&str)->Result<&'a str,String> {dict.get(key).map(Value::text).transpose().map(|s|s.unwrap_or(""))}
fn boolean(dict:&BTreeMap<String,Value>,key:&str)->Result<bool,String> {match dict.get(key) {None=>Ok(false),Some(Value::Bool(v))=>Ok(*v),_=>Err(format!("Apple field {key} must be boolean"))}}

pub(super) fn parse(bytes:&[u8],check:&impl Fn()->bool)->Result<Vec<RawPlaylist>,String> {
    let source=utf8(bytes)?;let mut reader=Reader::from_str(&source);reader.config_mut().expand_empty_elements=true;
    let mut parser=Parser{reader,check,nodes:0,text:0};let mut declared=false;let mut doctype=false;
    let root=loop {match parser.significant()? {
        Event::Decl(decl) if !declared && !doctype=>{declared=true;if decl.version().map_err(|e|e.to_string())?.as_ref()!=b"1.0" {return Err("Apple XML requires XML 1.0".into());}if let Some(encoding)=decl.encoding() {if !encoding.map_err(|e|e.to_string())?.eq_ignore_ascii_case(b"utf-8") {return Err("Apple XML declaration has unsupported encoding".into());}}},
        Event::DocType(value) if !doctype=>{doctype=true;let text=value.decode().map_err(|e|e.to_string())?;if text.contains('[') || text.contains(']') || !text.trim_start().starts_with("plist ") {return Err("XML entity declarations and internal DTDs are unsupported".into());}},
        Event::Start(start) if start.name().as_ref()==b"plist"=>{for attr in start.attributes() {let attr=attr.map_err(|e|e.to_string())?;if attr.key.as_ref()!=b"version" || attr.value.as_ref()!=b"1.0" {return Err("Unsupported Apple plist version or attribute".into());}}break start;},
        _=>return Err("Expected an Apple XML plist export".into()),
    }};drop(root);
    let event=parser.significant()?;let value=parser.value(event,0)?;
    if !matches!(parser.significant()?,Event::End(e) if e.name().as_ref()==b"plist") || !matches!(parser.significant()?,Event::Eof) {return Err("Trailing or incomplete Apple XML".into());}
    let root=value.dict()?;
    let records=root.get("Tracks").ok_or("Apple export has no Tracks dictionary")?.dict()?;
    if records.len()>100000 {return Err("Apple export exceeds 100000 track records".into());}
    let mut tracks=HashMap::new();
    for (key,value) in records {
        active(check)?;let dict=value.dict()?;let id=dict.get("Track ID").ok_or("Apple track has no Track ID")?.number()?;
        if key.parse::<u64>().ok()!=Some(id) || tracks.contains_key(&id) {return Err("Apple track identity is duplicated or inconsistent".into());}
        let kind=text(dict,"Kind")?.to_ascii_lowercase();let track_type=text(dict,"Track Type")?;
        let reference=label(text(dict,"Location")?)?;
        let blocked=if boolean(dict,"Protected")? || kind.contains("protected") {Some("Protected Apple media; unprotected local audio is required")}
            else if kind.contains("apple music") || track_type.eq_ignore_ascii_case("remote") || track_type.eq_ignore_ascii_case("url") {Some("Provider-only Apple entry; local unprotected audio is required")}
            else if reference.is_empty() {Some("Unmapped Apple entry: no local Location in the export")}else{None};
        tracks.insert(id,RawEntry{reference,title:label(text(dict,"Name")?)?,artist:label(text(dict,"Artist")?)?,blocked,details:Default::default()});
    }
    let playlists=root.get("Playlists").ok_or("Apple export has no Playlists array; export a playlist or library as XML")?.array()?;
    let mut result=Vec::new();let mut count=0;
    for (index,playlist) in playlists.iter().enumerate() {
        active(check)?;let dict=playlist.dict()?;let folder=boolean(dict,"Folder")?;
        if result.len()>=MAX_PLAYLISTS {return Err("Apple export exceeds 128 playlists and folders".into());}
        let mut entries=Vec::new();
        if let Some(items)=dict.get("Playlist Items") {for item in items.array()? {
            active(check)?;count+=1;if count>MAX_REFERENCES {return Err("Apple export exceeds 4096 playlist references; export a smaller selection".into());}
            let id=item.dict()?.get("Track ID").ok_or("Apple playlist item has no Track ID")?.number()?;
            entries.push(tracks.get(&id).cloned().unwrap_or(RawEntry{reference:format!("Apple track ID {id}"),title:String::new(),artist:String::new(),blocked:Some("Unmapped Apple playlist item: Track ID is absent from Tracks"),details:Default::default()}));
        }}
        let smart=dict.contains_key("Smart Info") || dict.contains_key("Smart Criteria");
        let persistent=text(dict,"Playlist Persistent ID")?;
        let key=if !persistent.is_empty() {format!("apple:{persistent}")}else{format!("apple:index:{index}")};
        let parent=text(dict,"Parent Persistent ID")?;let parent=(!parent.is_empty()).then(||format!("apple:{parent}"));
        result.push(RawPlaylist{name:name(text(dict,"Name")?)?,entries,folders:Vec::new(),key,parent,folder,note:if smart {"Smart playlist imported as a static snapshot of exported items"}else{"Apple playlist order and organizational folders retained"}.into()});
    }
    hierarchy(result)
}

fn hierarchy(mut playlists: Vec<RawPlaylist>) -> Result<Vec<RawPlaylist>,String> {
    let indices:HashMap<_,_>=playlists.iter().enumerate().map(|(i,p)|(p.key.clone(),i)).collect();
    if indices.len()!=playlists.len() {return Err("Apple export repeats a playlist identity".into());}
    let mut children=vec![Vec::new();playlists.len()+1];
    for i in 0..playlists.len() {
        let mut parent=playlists[i].parent.clone();let mut folders=Vec::new();let mut seen=HashSet::from([i]);
        while let Some(key)=parent {
            let index=*indices.get(&key).ok_or("Apple playlist parent is absent from the export")?;
            if !playlists[index].folder || !seen.insert(index) || folders.len()>=31 {return Err("Apple playlist folders are cyclic, invalid or exceed 32 levels".into());}
            folders.push(playlists[index].name.clone());parent=playlists[index].parent.clone();
        }
        folders.reverse();playlists[i].folders=folders;
        let parent=playlists[i].parent.as_ref().map(|key|indices[key]).unwrap_or(playlists.len());children[parent].push(i);
    }
    fn visit(parent:usize,children:&[Vec<usize>],order:&mut Vec<usize>) {for &child in &children[parent] {order.push(child);visit(child,children,order);}}
    let mut order=Vec::new();visit(playlists.len(),&children,&mut order);
    let mut nodes:Vec<_>=playlists.into_iter().map(Some).collect();Ok(order.into_iter().map(|i|nodes[i].take().unwrap()).collect())
}
