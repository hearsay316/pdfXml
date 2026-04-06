
use lopdf::{Document, Object};

fn main() {
    let doc = Document::load("examples/out.pdf").unwrap();
    let pages = doc.get_pages();
    let page_id = pages.get(&9).unwrap();
    let page = doc.get_object(*page_id).unwrap().as_dict().unwrap();
    println!("page object {:?}", page_id);
    let annots = match page.get(b"Annots") {
        Ok(Object::Array(items)) => items,
        other => {
            println!("annots missing: {:?}", other);
            return;
        }
    };
    println!("annots count {}", annots.len());
    for (i, item) in annots.iter().enumerate() {
        let id = match item.as_reference() { Ok(id) => id, Err(_) => continue };
        let obj = doc.get_object(id).unwrap();
        let dict = obj.as_dict().unwrap();
        let subtype = dict.get(b"Subtype").ok();
        let rect = dict.get(b"Rect").ok();
        let subj = dict.get(b"Subj").ok();
        let nm = dict.get(b"NM").ok().or_else(|| dict.get(b"Name").ok());
        let ap = dict.get(b"AP").ok();
        println!("annot {} id {:?} subtype {:?} rect {:?} subj {:?} name {:?} has_ap {}",
            i, id, subtype, rect, subj, nm, ap.is_some());
        if let Some(Object::Dictionary(apd)) = ap {
            println!("  ap keys {:?}", apd.iter().map(|(k, _)| String::from_utf8_lossy(k).to_string()).collect::<Vec<_>>());
        }
    }
}
