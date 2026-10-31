//! Native cloud clock math. No host-local timezone or wall clock is consulted.
use std::collections::BTreeSet;
use chrono::{Datelike, Duration, LocalResult, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;

fn field(raw: &str, min: u32, max: u32) -> Option<BTreeSet<u32>> {
    let mut values = BTreeSet::new();
    for part in raw.split(',') {
        let pieces = part.split('/').collect::<Vec<_>>();
        if pieces.len() > 2 { return None; }
        let step = if pieces.len() == 2 { pieces[1].parse::<u32>().ok()? } else { 1 };
        if step == 0 { return None; }
        let (start,end) = if pieces[0] == "*" { (min,max) }
        else if let Some((a,b)) = pieces[0].split_once('-') { (a.parse().ok()?,b.parse().ok()?) }
        else { let a = pieces[0].parse().ok()?; (a,if pieces.len()==2 {max} else {a}) };
        if start < min || end > max || start > end { return None; }
        for value in (start..=end).step_by(step as usize) { values.insert(value); }
    }
    (!values.is_empty()).then_some(values)
}

pub fn next_run(schedule: &str, after_ms: i64) -> Option<i64> {
    if !(0..=9_007_199_254_740_991).contains(&after_ms) { return None; }
    let normalized = schedule.split_whitespace().collect::<Vec<_>>().join(" ");
    let (expression,zone) = if let Some(rest) = normalized.strip_prefix("CRON_TZ=").or_else(||normalized.strip_prefix("TZ=")) {
        let (zone,expression)=rest.split_once(' ')?; (expression,zone.parse::<Tz>().ok()?)
    } else { (normalized.as_str(),chrono_tz::UTC) };
    let expression_lower = expression.to_ascii_lowercase();
    if let Some(rest)=expression_lower.strip_prefix("@every ") {
        let split=rest.find(|c:char|!c.is_ascii_digit())?;
        let amount=rest[..split].parse::<i64>().ok()?;
        if amount<=0 { return None; }
        let unit=match &rest[split..] {"s"=>1000,"m"=>60000,"h"=>3600000,"d"=>86400000,_=>return None};
        let next=after_ms.checked_add(amount.checked_mul(unit)?)?;
        return (next<=9_007_199_254_740_991).then_some(next);
    }
    let cron=match expression_lower.as_str() {
        "@hourly"=>"0 * * * *", "@daily"|"@midnight"=>"0 0 * * *", "@weekly"=>"0 0 * * 0",
        "@monthly"=>"0 0 1 * *", "@yearly"|"@annually"=>"0 0 1 1 *", _=>expression,
    };
    let fields=cron.split_whitespace().collect::<Vec<_>>();
    if fields.len()!=5 { return None; }
    let minutes=field(fields[0],0,59)?; let hours=field(fields[1],0,23)?;
    let days=field(fields[2],1,31)?; let months=field(fields[3],1,12)?;
    let weekdays=field(fields[4],0,7)?.into_iter().map(|d|d%7).collect::<BTreeSet<_>>();
    let start=Utc.timestamp_millis_opt(after_ms).single()?.with_timezone(&zone).date_naive();
    for offset in 0..=8*366 {
        let date=start.checked_add_signed(Duration::days(offset))?;
        if !months.contains(&date.month()) { continue; }
        let dom=days.contains(&date.day()); let dow=weekdays.contains(&date.weekday().num_days_from_sunday());
        let day_matches=match (fields[2]!="*",fields[4]!="*") {(true,true)=>dom||dow,(true,false)=>dom,(false,true)=>dow,_=>true};
        if !day_matches { continue; }
        let mut earliest=None;
        for hour in &hours { for minute in &minutes {
            let naive=date.and_time(NaiveTime::from_hms_opt(*hour,*minute,0)?);
            let candidates=match zone.from_local_datetime(&naive) {
                LocalResult::None=>vec![], LocalResult::Single(a)=>vec![a], LocalResult::Ambiguous(a,b)=>vec![a,b],
            };
            for candidate in candidates {
                let ms=candidate.timestamp_millis();
                if ms>after_ms && earliest.is_none_or(|old|ms<old) { earliest=Some(ms); }
            }
        }}
        if earliest.is_some() { return earliest; }
    }
    None
}

#[cfg(test)] mod tests {
    use super::*;
    fn ms(value:&str)->i64 { chrono::DateTime::parse_from_rfc3339(value).unwrap().timestamp_millis() }
    #[test] fn intervals_and_invalid_schedules_are_bounded() {
        assert_eq!(next_run("@every 1s",1000),Some(2000));
        assert_eq!(next_run("@EVERY 2h",1000),Some(7201000));
        for raw in ["", "@every 0s", "@every -1s", "@every 1ms", "*/0 * * * *", "60 * * * *", "CRON_TZ=Invalid/Zone @daily"] {assert_eq!(next_run(raw,0),None,"{raw}");}
    }
    #[test] fn timezone_dst_and_leap_days_use_actual_utc_occurrences() {
        let raw="CRON_TZ=America/New_York 30 1 * * *";
        assert_eq!(next_run(raw,ms("2026-11-01T05:30:00Z")),Some(ms("2026-11-01T06:30:00Z")));
        assert_eq!(next_run("TZ=America/New_York 30 2 * * *",ms("2026-03-08T00:00:00Z")),Some(ms("2026-03-09T06:30:00Z")));
        assert_eq!(next_run("0 0 29 2 *",ms("2028-03-01T00:00:00Z")),Some(ms("2032-02-29T00:00:00Z")));
        assert_eq!(next_run("CRON_TZ=Asia/Shanghai @daily",ms("2026-10-24T00:00:00Z")),Some(ms("2026-10-24T16:00:00Z")));
    }
    #[test] fn lists_steps_ranges_and_day_or_weekday_keep_cron_semantics() {
        assert_eq!(next_run("15,45 1-3/2 * * *",ms("2026-10-24T01:45:00Z")),Some(ms("2026-10-24T03:15:00Z")));
        assert_eq!(next_run("0 0 25 * 0",ms("2026-10-24T00:00:00Z")),Some(ms("2026-10-25T00:00:00Z")));
    }
}
