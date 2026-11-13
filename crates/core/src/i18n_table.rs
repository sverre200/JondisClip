//! (English, bokmål, nynorsk) for `tr!` (see i18n.rs, ADR-0018). Keep every
//! `{placeholder}`; a test checks that every `tr!` in the workspace is here.

pub static TABLE: &[(&str, &str, &str)] = &[
    // ---- tray: menu, tooltip, notifications
    (
        r#"Buffering last {length}"#,
        r#"Tar vare på siste {length}"#,
        r#"Tek vare på siste {length}"#,
    ),
    (r#"Paused"#, r#"På pause"#, r#"På pause"#),
    (r#"Sleeping"#, r#"Hviler"#, r#"Kviler"#),
    (
        r#"Paused: low disk space"#,
        r#"På pause: lite diskplass"#,
        r#"På pause: lite diskplass"#,
    ),
    (
        r#"Restarting ({reason})"#,
        r#"Starter på nytt ({reason})"#,
        r#"Startar på nytt ({reason})"#,
    ),
    (
        r#"resuming from sleep"#,
        r#"våkner fra hvilemodus"#,
        r#"vaknar frå kvilemodus"#,
    ),
    (
        r#"display changed"#,
        r#"skjermen endret seg"#,
        r#"skjermen endra seg"#,
    ),
    (
        r#"settings changed"#,
        r#"innstillingene endret seg"#,
        r#"innstillingane endra seg"#,
    ),
    (
        r#"microphone setting changed"#,
        r#"mikrofoninnstillingen endret seg"#,
        r#"mikrofoninnstillinga endra seg"#,
    ),
    (
        r#"hotkey unavailable"#,
        r#"hurtigtasten er utilgjengelig"#,
        r#"snøggtasten er utilgjengeleg"#,
    ),
    (
        r#"JondisClip is already running"#,
        r#"JondisClip kjører allerede"#,
        r#"JondisClip køyrer allereie"#,
    ),
    (
        r#"Right-click the tray icon for options."#,
        r#"Høyreklikk ikonet i systemstatusfeltet for valg."#,
        r#"Høgreklikk på ikonet i systemstatusfeltet for val."#,
    ),
    (
        r#"Save hotkey unavailable"#,
        r#"Hurtigtasten for lagring er utilgjengelig"#,
        r#"Snøggtasten for lagring er utilgjengeleg"#,
    ),
    (
        r#"{key} is used by another program. Choose another hotkey in settings."#,
        r#"{key} brukes av et annet program. Velg en annen hurtigtast i innstillingene."#,
        r#"{key} vert brukt av eit anna program. Vel ein annan snøggtast i innstillingane."#,
    ),
    (
        r#"Recording without audio"#,
        r#"Tar opp uten lyd"#,
        r#"Tek opp utan lyd"#,
    ),
    (
        r#"Audio capture couldn't start. Video is still recorded."#,
        r#"Lydopptaket kunne ikke starte. Video tas fortsatt opp."#,
        r#"Lydopptaket kunne ikkje starte. Video vert framleis teken opp."#,
    ),
    (
        r#"JondisClip can't record"#,
        r#"JondisClip kan ikke ta opp"#,
        r#"JondisClip kan ikkje ta opp"#,
    ),
    (
        r#"{error}. Retrying automatically."#,
        r#"{error}. Prøver igjen automatisk."#,
        r#"{error}. Prøver igjen automatisk."#,
    ),
    (
        r#"Buffering paused: low disk space"#,
        r#"Bufring satt på pause: lite diskplass"#,
        r#"Bufring sett på pause: lite diskplass"#,
    ),
    (
        r#"Free up some space and JondisClip will resume automatically."#,
        r#"Frigjør litt plass, så fortsetter JondisClip automatisk."#,
        r#"Frigjer litt plass, så held JondisClip fram automatisk."#,
    ),
    (
        r#"Clip not saved"#,
        r#"Klippet ble ikke lagret"#,
        r#"Klippet vart ikkje lagra"#,
    ),
    (
        r#"JondisClip is quitting after the clip it's still saving."#,
        r#"JondisClip avslutter etter klippet som fortsatt lagres."#,
        r#"JondisClip avsluttar etter klippet som framleis vert lagra."#,
    ),
    (
        r#"JondisClip isn't buffering ({status})."#,
        r#"JondisClip bufrer ikke ({status})."#,
        r#"JondisClip bufrar ikkje ({status})."#,
    ),
    (
        r#"Can't write to {path}: {error}"#,
        r#"Kan ikke skrive til {path}: {error}"#,
        r#"Kan ikkje skrive til {path}: {error}"#,
    ),
    (
        r#"Finishing your clip"#,
        r#"Gjør ferdig klippet ditt"#,
        r#"Gjer ferdig klippet ditt"#,
    ),
    (
        r#"JondisClip quits as soon as the clip is saved."#,
        r#"JondisClip avslutter så snart klippet er lagret."#,
        r#"JondisClip avsluttar så snart klippet er lagra."#,
    ),
    (
        r#"Clip saved · {game} · {length}"#,
        r#"Klipp lagret · {game} · {length}"#,
        r#"Klipp lagra · {game} · {length}"#,
    ),
    (
        r#"Couldn't save clip"#,
        r#"Kunne ikke lagre klippet"#,
        r#"Kunne ikkje lagre klippet"#,
    ),
    (
        r#"Save last {length}"#,
        r#"Lagre siste {length}"#,
        r#"Lagre siste {length}"#,
    ),
    (
        r#"Resume buffering"#,
        r#"Fortsett bufring"#,
        r#"Hald fram med bufring"#,
    ),
    (
        r#"Pause buffering"#,
        r#"Sett bufring på pause"#,
        r#"Set bufring på pause"#,
    ),
    (
        r#"Replay length"#,
        r#"Lengde på reprisen"#,
        r#"Lengd på reprisen"#,
    ),
    (
        r#"Record microphone"#,
        r#"Ta opp mikrofon"#,
        r#"Ta opp mikrofon"#,
    ),
    (r#"Clip library"#, r#"Klippbibliotek"#, r#"Klippbibliotek"#),
    (
        r#"Open clips folder"#,
        r#"Åpne klippmappen"#,
        r#"Opne klippmappa"#,
    ),
    (r#"Settings…"#, r#"Innstillinger …"#, r#"Innstillingar …"#),
    (
        r#"Quit JondisClip"#,
        r#"Avslutt JondisClip"#,
        r#"Avslutt JondisClip"#,
    ),
    (
        r#"Couldn't open JondisClip"#,
        r#"Kunne ikke åpne JondisClip"#,
        r#"Kunne ikkje opne JondisClip"#,
    ),
    (
        r#"JondisClip window missing"#,
        r#"JondisClip-vinduet mangler"#,
        r#"JondisClip-vindauget manglar"#,
    ),
    (
        r#"jondisclip-ui.exe wasn't found next to jondisclip.exe."#,
        r#"Fant ikke jondisclip-ui.exe ved siden av jondisclip.exe."#,
        r#"Fann ikkje jondisclip-ui.exe ved sida av jondisclip.exe."#,
    ),
    // ---- window: settings
    (
        r#"The clips folder and the Google Drive folder can't overlap. Choose separate folders."#,
        r#"Klippmappen og Google Drive-mappen kan ikke overlappe. Velg separate mapper."#,
        r#"Klippmappa og Google Drive-mappa kan ikkje overlappe. Vel separate mapper."#,
    ),
    (
        r#"The Google Drive folder can't be inside JondisClip's own folders."#,
        r#"Google Drive-mappen kan ikke ligge i JondisClips egne mapper."#,
        r#"Google Drive-mappa kan ikkje liggje i JondisClips eigne mapper."#,
    ),
    (
        r#"couldn't save settings: {error}"#,
        r#"kunne ikke lagre innstillingene: {error}"#,
        r#"kunne ikkje lagre innstillingane: {error}"#,
    ),
    (
        r#"The check didn't finish."#,
        r#"Sjekken ble ikke ferdig."#,
        r#"Sjekken vart ikkje ferdig."#,
    ),
    (
        r#"The Google Drive folder isn't available right now."#,
        r#"Google Drive-mappen er ikke tilgjengelig akkurat nå."#,
        r#"Google Drive-mappa er ikkje tilgjengeleg akkurat no."#,
    ),
    (
        r#"Windows couldn't open the clip (no video player set?)"#,
        r#"Windows kunne ikke åpne klippet (ingen videospiller valgt?)"#,
        r#"Windows kunne ikkje opne klippet (ingen videospelar vald?)"#,
    ),
    (
        r#"the clipboard is busy; try again"#,
        r#"utklippstavlen er opptatt; prøv igjen"#,
        r#"utklippstavla er oppteken; prøv igjen"#,
    ),
    // ---- window: library, folders, files
    (
        r#"{name} is in use (playing or being exported?), so its file keeps the old name."#,
        r#"{name} er i bruk (spilles av eller eksporteres?), så filen beholder det gamle navnet."#,
        r#"{name} er i bruk (vert spela av eller eksportert?), så fila beheld det gamle namnet."#,
    ),
    (
        r#"Windows denied renaming {name} (permissions)."#,
        r#"Windows nektet å gi {name} nytt navn (tillatelser)."#,
        r#"Windows nekta å gje {name} nytt namn (løyve)."#,
    ),
    (
        r#"couldn't rename {name}: {error}"#,
        r#"kunne ikke gi {name} nytt navn: {error}"#,
        r#"kunne ikkje gje {name} nytt namn: {error}"#,
    ),
    (
        r#"The clips folder isn't available: {error}"#,
        r#"Klippmappen er ikke tilgjengelig: {error}"#,
        r#"Klippmappa er ikkje tilgjengeleg: {error}"#,
    ),
    (
        r#"{path} is outside the clips folder"#,
        r#"{path} ligger utenfor klippmappen"#,
        r#"{path} ligg utanfor klippmappa"#,
    ),
    (
        r#"couldn't create {path}: {error}"#,
        r#"kunne ikke opprette {path}: {error}"#,
        r#"kunne ikkje opprette {path}: {error}"#,
    ),
    (
        r#"{name} is in use (playing or being exported?). Close it and try again."#,
        r#"{name} er i bruk (spilles av eller eksporteres?). Lukk det og prøv igjen."#,
        r#"{name} er i bruk (vert spela av eller eksportert?). Lukk det og prøv igjen."#,
    ),
    (
        r#"Windows denied moving {name} (permissions)."#,
        r#"Windows nektet å flytte {name} (tillatelser)."#,
        r#"Windows nekta å flytte {name} (løyve)."#,
    ),
    (
        r#"{path} already exists; nothing was overwritten."#,
        r#"{path} finnes allerede; ingenting ble overskrevet."#,
        r#"{path} finst allereie; ingenting vart skrive over."#,
    ),
    (
        r#"couldn't move {name}: {error}"#,
        r#"kunne ikke flytte {name}: {error}"#,
        r#"kunne ikkje flytte {name}: {error}"#,
    ),
    (
        r#"couldn't copy: {error}"#,
        r#"kunne ikke kopiere: {error}"#,
        r#"kunne ikkje kopiere: {error}"#,
    ),
    (
        r#"the copy didn't verify; the original is untouched"#,
        r#"kopien kunne ikke bekreftes; originalen er urørt"#,
        r#"kopien kunne ikkje stadfestast; originalen er urørt"#,
    ),
    (
        r#"couldn't finish the copy: {error}"#,
        r#"kunne ikke fullføre kopien: {error}"#,
        r#"kunne ikkje fullføre kopien: {error}"#,
    ),
    (
        r#"Copied to {path}, but the original couldn't be removed ({error}); you now have both."#,
        r#"Kopiert til {path}, men originalen kunne ikke fjernes ({error}); nå har du begge."#,
        r#"Kopiert til {path}, men originalen kunne ikkje fjernast ({error}); no har du begge."#,
    ),
    (
        r#"{path} changed since the library was loaded. Refresh and try again."#,
        r#"{path} er endret siden biblioteket ble lastet. Oppdater og prøv igjen."#,
        r#"{path} er endra sidan biblioteket vart lasta. Oppdater og prøv igjen."#,
    ),
    (
        r#"Wait for the running export to finish first."#,
        r#"Vent til eksporten som kjører er ferdig først."#,
        r#"Vent til eksporten som køyrer er ferdig først."#,
    ),
    (
        r#"That folder no longer exists. Refresh and try again."#,
        r#"Den mappen finnes ikke lenger. Oppdater og prøv igjen."#,
        r#"Den mappa finst ikkje lenger. Oppdater og prøv igjen."#,
    ),
    (
        r#"That folder no longer exists."#,
        r#"Den mappen finnes ikke lenger."#,
        r#"Den mappa finst ikkje lenger."#,
    ),
    (
        r#"{path} isn't on disk any more. Refresh and try again."#,
        r#"{path} finnes ikke lenger på disken. Oppdater og prøv igjen."#,
        r#"{path} finst ikkje lenger på disken. Oppdater og prøv igjen."#,
    ),
    (
        r#"The clips folder {path} isn't available."#,
        r#"Klippmappen {path} er ikke tilgjengelig."#,
        r#"Klippmappa {path} er ikkje tilgjengeleg."#,
    ),
    (
        r#"There's already something called "{name}" in {path}."#,
        r#"Det finnes allerede noe som heter «{name}» i {path}."#,
        r#"Det finst allereie noko som heiter «{name}» i {path}."#,
    ),
    (
        r#"couldn't create the folder: {error}"#,
        r#"kunne ikke opprette mappen: {error}"#,
        r#"kunne ikkje opprette mappa: {error}"#,
    ),
    (
        r#"A clip in this folder is in use (playing or exporting?). Close it and try again."#,
        r#"Et klipp i denne mappen er i bruk (spilles av eller eksporteres?). Lukk det og prøv igjen."#,
        r#"Eit klipp i denne mappa er i bruk (vert spela av eller eksportert?). Lukk det og prøv igjen."#,
    ),
    (
        r#"Folders can't be moved to another drive."#,
        r#"Mapper kan ikke flyttes til en annen disk."#,
        r#"Mapper kan ikkje flyttast til ein annan disk."#,
    ),
    (
        r#"couldn't move the folder: {error}"#,
        r#"kunne ikke flytte mappen: {error}"#,
        r#"kunne ikkje flytte mappa: {error}"#,
    ),
    (
        r#"A folder can't go inside itself."#,
        r#"En mappe kan ikke ligge inni seg selv."#,
        r#"Ei mappe kan ikkje liggje inni seg sjølv."#,
    ),
    (
        r#"Folders go up to {n} levels deep."#,
        r#"Mapper kan ligge maks {n} nivåer dypt."#,
        r#"Mapper kan liggje maks {n} nivå djupt."#,
    ),
    (
        r#""{name}" is a game folder name at the top level. Rename the folder first."#,
        r#"«{name}» er navnet på en spillmappe på øverste nivå. Gi mappen nytt navn først."#,
        r#"«{name}» er namnet på ei spelmappe på øvste nivå. Gje mappa nytt namn først."#,
    ),
    (
        r#"That folder isn't on disk any more. Refresh and try again."#,
        r#"Den mappen finnes ikke lenger på disken. Oppdater og prøv igjen."#,
        r#"Den mappa finst ikkje lenger på disken. Oppdater og prøv igjen."#,
    ),
    (
        r#"couldn't open File Explorer: {error}"#,
        r#"kunne ikke åpne Filutforsker: {error}"#,
        r#"kunne ikkje opne Filutforskar: {error}"#,
    ),
    (
        r#"Title saved, but {error}"#,
        r#"Tittelen ble lagret, men {error}"#,
        r#"Tittelen vart lagra, men {error}"#,
    ),
    (
        r#"There's a file called {name} in your clips folder, so the folder can't be made."#,
        r#"Det finnes en fil som heter {name} i klippmappen din, så mappen kan ikke opprettes."#,
        r#"Det finst ei fil som heiter {name} i klippmappa di, så mappa kan ikkje opprettast."#,
    ),
    (
        r#"Couldn't make the folder {path}: {error}"#,
        r#"Kunne ikke opprette mappen {path}: {error}"#,
        r#"Kunne ikkje opprette mappa {path}: {error}"#,
    ),
    (r#"cancelled"#, r#"avbrutt"#, r#"avbroten"#),
    (
        r#"a file was in use or couldn't be recycled"#,
        r#"en fil var i bruk eller kunne ikke flyttes til papirkurven"#,
        r#"ei fil var i bruk eller kunne ikkje flyttast til papirkorga"#,
    ),
    (
        r#"Nothing was deleted ({why})."#,
        r#"Ingenting ble slettet ({why})."#,
        r#"Ingenting vart sletta ({why})."#,
    ),
    (
        r#"{gone} of {total} clips were moved to the Recycle Bin ({why})."#,
        r#"{gone} av {total} klipp ble flyttet til papirkurven ({why})."#,
        r#"{gone} av {total} klipp vart flytte til papirkorga ({why})."#,
    ),
    (
        r#"{gone} of {total} clips were deleted ({why})."#,
        r#"{gone} av {total} klipp ble slettet ({why})."#,
        r#"{gone} av {total} klipp vart sletta ({why})."#,
    ),
    // ---- window: export, share, Drive copies
    (
        r#"Choose a section to save: the start must be before the end."#,
        r#"Velg en del å lagre: starten må være før slutten."#,
        r#"Vel ein del å lagre: starten må vere før slutten."#,
    ),
    (
        r#"An export is already running. Wait for it or cancel it first."#,
        r#"En eksport kjører allerede. Vent på den eller avbryt den først."#,
        r#"Ein eksport køyrer allereie. Vent på han eller avbryt han først."#,
    ),
    (
        r#"The save folder overlaps JondisClip's share folder ({path}); choose another save folder."#,
        r#"Lagringsmappen overlapper JondisClips delingsmappe ({path}); velg en annen lagringsmappe."#,
        r#"Lagringsmappa overlappar JondisClips delingsmappe ({path}); vel ei anna lagringsmappe."#,
    ),
    (
        r#"couldn't set up the export safely (job object); try again"#,
        r#"kunne ikke sette opp eksporten trygt (jobbobjekt); prøv igjen"#,
        r#"kunne ikkje setje opp eksporten trygt (jobbobjekt); prøv igjen"#,
    ),
    (
        r#"couldn't start the exporter: {error}"#,
        r#"kunne ikke starte eksportøren: {error}"#,
        r#"kunne ikkje starte eksportøren: {error}"#,
    ),
    (
        r#"Not enough free space on {path} for Google Drive's local cache (about {mb} MB needed, plus 2 GB kept free for recording)."#,
        r#"Ikke nok ledig plass på {path} for den lokale hurtigbufferen til Google Drive (rundt {mb} MB trengs, pluss 2 GB holdes ledig for opptak)."#,
        r#"Ikkje nok ledig plass på {path} for den lokale mellombufferen til Google Drive (rundt {mb} MB trengst, pluss 2 GB vert haldne ledige for opptak)."#,
    ),
    (
        r#"Not enough free space on {path} (about {mb} MB needed, plus {reserve} kept free)."#,
        r#"Ikke nok ledig plass på {path} (rundt {mb} MB trengs, pluss {reserve} holdes ledig)."#,
        r#"Ikkje nok ledig plass på {path} (rundt {mb} MB trengst, pluss {reserve} vert haldne ledige)."#,
    ),
    (
        r#"{path} already exists"#,
        r#"{path} finnes allerede"#,
        r#"{path} finst allereie"#,
    ),
    (
        r#"couldn't read the clip"#,
        r#"kunne ikke lese klippet"#,
        r#"kunne ikkje lese klippet"#,
    ),
    (
        r#"couldn't write to the Drive folder"#,
        r#"kunne ikke skrive til Drive-mappen"#,
        r#"kunne ikkje skrive til Drive-mappa"#,
    ),
    (
        r#"couldn't finish the file"#,
        r#"kunne ikke fullføre filen"#,
        r#"kunne ikkje fullføre fila"#,
    ),
    (
        r#"couldn't check the copy"#,
        r#"kunne ikke sjekke kopien"#,
        r#"kunne ikkje sjekke kopien"#,
    ),
    (
        r#"The copy came out the wrong size ({copied} of {size} bytes). Nothing was saved; try again."#,
        r#"Kopien fikk feil størrelse ({copied} av {size} byte). Ingenting ble lagret; prøv igjen."#,
        r#"Kopien fekk feil storleik ({copied} av {size} byte). Ingenting vart lagra; prøv igjen."#,
    ),
    (
        r#"The copy didn't match the clip. Nothing was saved; try again."#,
        r#"Kopien stemte ikke med klippet. Ingenting ble lagret; prøv igjen."#,
        r#"Kopien stemde ikkje med klippet. Ingenting vart lagra; prøv igjen."#,
    ),
    (
        r#"The export stopped unexpectedly."#,
        r#"Eksporten stoppet uventet."#,
        r#"Eksporten stoppa uventa."#,
    ),
    // ---- exporter
    (
        r#"FFmpeg couldn't {what}: {error}"#,
        r#"FFmpeg kunne ikke {what}: {error}"#,
        r#"FFmpeg kunne ikkje {what}: {error}"#,
    ),
    (
        r#"couldn't write the file: {error}"#,
        r#"kunne ikke skrive filen: {error}"#,
        r#"kunne ikkje skrive fila: {error}"#,
    ),
    (
        r#"the clip has no video"#,
        r#"klippet har ingen video"#,
        r#"klippet har ingen video"#,
    ),
    (
        r#"the selection is empty (start must be before end)"#,
        r#"utvalget er tomt (starten må være før slutten)"#,
        r#"utvalet er tomt (starten må vere før slutten)"#,
    ),
    (
        r#"{path} already exists; originals and earlier exports are never overwritten"#,
        r#"{path} finnes allerede; originaler og tidligere eksporter overskrives aldri"#,
        r#"{path} finst allereie; originalar og tidlegare eksportar vert aldri skrivne over"#,
    ),
    (
        r#"no hardware H.264 encoder could be opened ({error})"#,
        r#"ingen H.264-maskinvarekoder kunne åpnes ({error})"#,
        r#"ingen H.264-maskinvarekodar kunne opnast ({error})"#,
    ),
    (
        r#"nothing was written for the selection"#,
        r#"ingenting ble skrevet for utvalget"#,
        r#"ingenting vart skrive for utvalet"#,
    ),
    (
        r#"the video's timestamps go backwards at {time} s; the clip can't be cut cleanly"#,
        r#"tidsstemplene i videoen går bakover ved {time} s; klippet kan ikke klippes rent"#,
        r#"tidsstempla i videoen går bakover ved {time} s; klippet kan ikkje klippast reint"#,
    ),
    (
        r#"this clip uses B-frames, so a lossless cut could break its last frames; use Exact"#,
        r#"klippet bruker B-bilder, så en tapsfri klipping kan ødelegge de siste bildene; bruk Nøyaktig"#,
        r#"klippet brukar B-bilete, så ei tapsfri klipping kan øydeleggje dei siste bileta; bruk Nøyaktig"#,
    ),
    (
        r#"the encoder stayed busy for too long (the GPU is fully loaded); try again"#,
        r#"koderen var opptatt for lenge (skjermkortet er fullt belastet); prøv igjen"#,
        r#"kodaren var oppteken for lenge (skjermkortet er fullt belasta); prøv igjen"#,
    ),
    (
        r#"too long for {mb} MB: trim it to under {s} s"#,
        r#"for langt for {mb} MB: klipp det til under {s} s"#,
        r#"for langt for {mb} MB: klipp det til under {s} s"#,
    ),
    (
        r#"couldn't get it under the limit ({size} MB, limit {limit} MB); trim it shorter"#,
        r#"fikk det ikke under grensen ({size} MB, grense {limit} MB); klipp det kortere"#,
        r#"fekk det ikkje under grensa ({size} MB, grense {limit} MB); klipp det kortare"#,
    ),
    // ---- core: folders, library file
    (
        r#"that folder no longer exists"#,
        r#"den mappen finnes ikke lenger"#,
        r#"den mappa finst ikkje lenger"#,
    ),
    (
        r#"Give the folder a name."#,
        r#"Gi mappen et navn."#,
        r#"Gje mappa eit namn."#,
    ),
    (
        r#"Folder names can be up to {n} characters."#,
        r#"Mappenavn kan ha opptil {n} tegn."#,
        r#"Mappenamn kan ha opptil {n} teikn."#,
    ),
    (
        r#"Folder names can't contain {char}"#,
        r#"Mappenavn kan ikke inneholde {char}"#,
        r#"Mappenamn kan ikkje innehalde {char}"#,
    ),
    (
        r#"Folder names can't end with a dot."#,
        r#"Mappenavn kan ikke slutte med punktum."#,
        r#"Mappenamn kan ikkje slutte med punktum."#,
    ),
    (
        r#""{name}" is reserved by Windows."#,
        r#"«{name}» er reservert av Windows."#,
        r#"«{name}» er reservert av Windows."#,
    ),
    (
        r#"There's already a folder called "{name}" there."#,
        r#"Det finnes allerede en mappe som heter «{name}» der."#,
        r#"Det finst allereie ei mappe som heiter «{name}» der."#,
    ),
    (
        r#"Titles can be up to {n} characters."#,
        r#"Titler kan ha opptil {n} tegn."#,
        r#"Titlar kan ha opptil {n} teikn."#,
    ),
    (
        r#"{path} is damaged: {error}"#,
        r#"{path} er skadet: {error}"#,
        r#"{path} er skadd: {error}"#,
    ),
    (
        r#"couldn't read {path}: {error}"#,
        r#"kunne ikke lese {path}: {error}"#,
        r#"kunne ikkje lese {path}: {error}"#,
    ),
    (
        r#"Your folders were saved by a newer JondisClip (format {v}); this version won't change them."#,
        r#"Mappene dine ble lagret av en nyere JondisClip (format {v}); denne versjonen endrer dem ikke."#,
        r#"Mappene dine vart lagra av ein nyare JondisClip (format {v}); denne versjonen endrar dei ikkje."#,
    ),
    (
        r#"{error}. Try again in a moment."#,
        r#"{error}. Prøv igjen om et øyeblikk."#,
        r#"{error}. Prøv igjen om eit augeblink."#,
    ),
    (
        r#"couldn't save the library: {error}"#,
        r#"kunne ikke lagre biblioteket: {error}"#,
        r#"kunne ikkje lagre biblioteket: {error}"#,
    ),
    // ---- core: Google Drive folder checks
    (
        r#"JondisClip isn't allowed to write there."#,
        r#"JondisClip har ikke lov til å skrive der."#,
        r#"JondisClip har ikkje lov til å skrive der."#,
    ),
    (
        r#"Writing there failed: {error}"#,
        r#"Skriving der mislyktes: {error}"#,
        r#"Skriving der mislukkast: {error}"#,
    ),
    (
        r#"A test file couldn't be removed again: {error}"#,
        r#"En testfil kunne ikke fjernes igjen: {error}"#,
        r#"Ei testfil kunne ikkje fjernast igjen: {error}"#,
    ),
    (
        r#"Choose your Google Drive folder in Settings → Google Drive first."#,
        r#"Velg Google Drive-mappen din i Innstillinger → Google Drive først."#,
        r#"Vel Google Drive-mappa di i Innstillingar → Google Drive først."#,
    ),
    (
        r#"Google Drive isn't available ({path} not found). Is Google Drive for desktop running and signed in?"#,
        r#"Google Drive er ikke tilgjengelig (fant ikke {path}). Kjører Google Drive for datamaskiner, og er du logget på?"#,
        r#"Google Drive er ikkje tilgjengeleg (fann ikkje {path}). Køyrer Google Drive for datamaskiner, og er du logga på?"#,
    ),
    (
        r#"The Drive folder overlaps your clips folder. Choose a separate folder for Drive."#,
        r#"Drive-mappen overlapper klippmappen din. Velg en egen mappe for Drive."#,
        r#"Drive-mappa overlappar klippmappa di. Vel ei eiga mappe for Drive."#,
    ),
    (
        r#"The Drive folder can't be inside JondisClip's own folders."#,
        r#"Drive-mappen kan ikke ligge i JondisClips egne mapper."#,
        r#"Drive-mappa kan ikkje liggje i JondisClips eigne mapper."#,
    ),
    // ---- core: hotkey
    (
        r#"hotkey is empty"#,
        r#"hurtigtasten er tom"#,
        r#"snøggtasten er tom"#,
    ),
    (
        r#"unknown key "{key}""#,
        r#"ukjent tast «{key}»"#,
        r#"ukjend tast «{key}»"#,
    ),
    (
        r#"hotkey has modifiers but no key"#,
        r#"hurtigtasten har modifikatorer, men ingen tast"#,
        r#"snøggtasten har modifikatorar, men ingen tast"#,
    ),
    (
        r#"hotkey has more than one key"#,
        r#"hurtigtasten har mer enn én tast"#,
        r#"snøggtasten har meir enn éin tast"#,
    ),
    (
        r#""{key}" needs Ctrl, Alt, Shift or Win"#,
        r#"«{key}» trenger Ctrl, Alt, Shift eller Win"#,
        r#"«{key}» treng Ctrl, Alt, Shift eller Win"#,
    ),
    // ---- core: mic check tips
    (
        r#"The microphone itself clipped, before JondisClip. Turn the G-Track's MIC GAIN knob down until this no longer appears. This distortion can't be fixed afterwards."#,
        r#"Selve mikrofonen overstyrte, før JondisClip. Skru ned MIC GAIN-knappen på G-Track til dette ikke lenger vises. Denne forvrengningen kan ikke fikses i etterkant."#,
        r#"Sjølve mikrofonen overstyrte, før JondisClip. Skru ned MIC GAIN-knappen på G-Track til dette ikkje lenger vert vist. Denne forvrenginga kan ikkje fiksast i etterkant."#,
    ),
    (
        r#"No speech found. Speak normally for the whole test, about 20-30 cm from the mic, into the front (logo side)."#,
        r#"Fant ingen tale. Snakk som vanlig gjennom hele testen, rundt 20–30 cm fra mikrofonen, inn i forsiden (logosiden)."#,
        r#"Fann ingen tale. Snakk som vanleg gjennom heile testen, rundt 20–30 cm frå mikrofonen, inn i framsida (logosida)."#,
    ),
    (
        r#"Your voice reaches the computer at {level} dBFS. Turn the G-Track's MIC GAIN knob up (aim for about {more} dB more) and set Microphone gain here back to 0 dB. A louder signal from the mic itself has less hiss than digital boost."#,
        r#"Stemmen din når datamaskinen på {level} dBFS. Skru opp MIC GAIN-knappen på G-Track (sikt på rundt {more} dB mer) og sett Mikrofonforsterkning her tilbake til 0 dB. Et sterkere signal fra selve mikrofonen gir mindre sus enn digital forsterkning."#,
        r#"Stemma di når datamaskina på {level} dBFS. Skru opp MIC GAIN-knappen på G-Track (sikt på rundt {more} dB meir) og set Mikrofonforsterking her tilbake til 0 dB. Eit sterkare signal frå sjølve mikrofonen gjev mindre sus enn digital forsterking."#,
    ),
    (
        r#"Peaks from the mic are already close to full scale, so raise the level here instead: set Microphone gain to about {gain} dB."#,
        r#"Toppene fra mikrofonen er allerede nær fullt utslag, så øk nivået her i stedet: sett Mikrofonforsterkning til rundt {gain} dB."#,
        r#"Toppane frå mikrofonen er allereie nær fullt utslag, så auk nivået her i staden: set Mikrofonforsterking til rundt {gain} dB."#,
    ),
    (
        r#"Your voice is louder than needed. Turn the MIC GAIN knob down a little (about {less} dB), or set Microphone gain to {gain} dB."#,
        r#"Stemmen din er sterkere enn nødvendig. Skru MIC GAIN-knappen litt ned (rundt {less} dB), eller sett Mikrofonforsterkning til {gain} dB."#,
        r#"Stemma di er sterkare enn naudsynt. Skru MIC GAIN-knappen litt ned (rundt {less} dB), eller set Mikrofonforsterking til {gain} dB."#,
    ),
    (
        r#"Microphone gain here is {gain} dB. With the knob set right, 0 dB is best."#,
        r#"Mikrofonforsterkningen her er {gain} dB. Med knappen riktig innstilt er 0 dB best."#,
        r#"Mikrofonforsterkinga her er {gain} dB. Med knappen rett innstilt er 0 dB best."#,
    ),
    (
        r#"Peaks hit the safety limiter for {ms} ms. Back off a little from the mic, or lower the gain."#,
        r#"Toppene traff sikkerhetsbegrenseren i {ms} ms. Gå litt unna mikrofonen, eller senk forsterkningen."#,
        r#"Toppane trefte tryggleiksavgrensaren i {ms} ms. Gå litt unna mikrofonen, eller senk forsterkinga."#,
    ),
    (
        r#"Background noise is fairly high ({level} dBFS). Get closer to the mic and use less gain, and point the back of the mic towards fans or the PC (cardioid pattern rejects sound from behind)."#,
        r#"Bakgrunnsstøyen er ganske høy ({level} dBFS). Kom nærmere mikrofonen og bruk mindre forsterkning, og vend baksiden av mikrofonen mot vifter eller PC-en (nyremønsteret demper lyd bakfra)."#,
        r#"Bakgrunnsstøyen er ganske høg ({level} dBFS). Kom nærare mikrofonen og bruk mindre forsterking, og vend baksida av mikrofonen mot vifter eller PC-en (nyremønsteret dempar lyd bakfrå)."#,
    ),
    (
        r#"Strong rumble below 60 Hz while you speak ({level} dB): proximity boom, desk bumps or breath pops. Turn on Low-cut filter."#,
        r#"Kraftig rumling under 60 Hz mens du snakker ({level} dB): nærhetsbass, dunk i bordet eller pustelyder. Slå på Lavkuttfilter."#,
        r#"Kraftig rumling under 60 Hz medan du snakkar ({level} dB): nærleiksbass, dunk i bordet eller pustelydar. Slå på Lågkuttfilter."#,
    ),
    (
        r#"The G-Track Pro is in 2-Track mode (mic on the left channel only). JondisClip handles it, but MONO mode is simpler and recommended: press the mode button so MONO is lit."#,
        r#"G-Track Pro er i 2-Track-modus (mikrofonen bare på venstre kanal). JondisClip takler det, men MONO-modus er enklere og anbefales: trykk på modusknappen så MONO lyser."#,
        r#"G-Track Pro er i 2-Track-modus (mikrofonen berre på venstre kanal). JondisClip taklar det, men MONO-modus er enklare og tilrådd: trykk på modusknappen så MONO lyser."#,
    ),
    (
        r#"Your microphone is set up well. No changes needed."#,
        r#"Mikrofonen din er godt innstilt. Ingen endringer trengs."#,
        r#"Mikrofonen din er godt innstilt. Ingen endringar trengst."#,
    ),
    // ---- updates (ADR-0020)
    (
        r#"Update available: JondisClip {version}"#,
        r#"Oppdatering tilgjengelig: JondisClip {version}"#,
        r#"Oppdatering tilgjengeleg: JondisClip {version}"#,
    ),
    (
        r#"Install it here, from the tray menu or in Settings. Recording continues while it downloads."#,
        r#"Installer den her, fra menyen i systemstatusfeltet eller i Innstillinger. Opptaket fortsetter mens den lastes ned."#,
        r#"Installer ho her, frå menyen i systemstatusfeltet eller i Innstillingar. Opptaket held fram medan ho vert lasta ned."#,
    ),
    (
        r#"Install update"#,
        r#"Installer oppdatering"#,
        r#"Installer oppdatering"#,
    ),
    (
        r#"This copy is a developer build, so it doesn't update itself. Rebuild it to get the new version."#,
        r#"Denne kopien er et utviklerbygg og oppdaterer ikke seg selv. Bygg den på nytt for å få den nye versjonen."#,
        r#"Denne kopien er eit utviklarbygg og oppdaterer ikkje seg sjølv. Bygg han på nytt for å få den nye versjonen."#,
    ),
    (
        r#"Version {version} is out (developer build: rebuild to update)"#,
        r#"Versjon {version} er ute (utviklerbygg: bygg på nytt for å oppdatere)"#,
        r#"Versjon {version} er ute (utviklarbygg: bygg på nytt for å oppdatere)"#,
    ),
    (
        r#"Open library"#,
        r#"Åpne biblioteket"#,
        r#"Opne biblioteket"#,
    ),
    (r#"Show in folder"#, r#"Vis i mappe"#, r#"Vis i mappe"#),
    (
        r#"Install update {version}"#,
        r#"Installer oppdatering {version}"#,
        r#"Installer oppdatering {version}"#,
    ),
    (
        r#"Downloading update…"#,
        r#"Laster ned oppdatering …"#,
        r#"Lastar ned oppdatering …"#,
    ),
    (
        r#"Downloading JondisClip {version}"#,
        r#"Laster ned JondisClip {version}"#,
        r#"Lastar ned JondisClip {version}"#,
    ),
    (
        r#"It installs and restarts by itself when it's ready. Recording continues meanwhile."#,
        r#"Den installeres og starter på nytt av seg selv når den er klar. Opptaket fortsetter imens."#,
        r#"Ho vert installert og startar på nytt av seg sjølv når ho er klar. Opptaket held fram imens."#,
    ),
    (
        r#"Couldn't update JondisClip"#,
        r#"Kunne ikke oppdatere JondisClip"#,
        r#"Kunne ikkje oppdatere JondisClip"#,
    ),
];
