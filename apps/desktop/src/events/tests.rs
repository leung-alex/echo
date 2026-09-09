use super::*;
use echo_engine::InlineTicket;
use echo_windows::inline::InlineEvent;
fn ticket(revision: u64) -> InlineTicket {
    InlineTicket {
        session: 7,
        revision,
        input_serial: revision,
    }
}
fn changed(revision: u64) -> Event {
    Event::Inline(InlineEvent::Changed {
        ticket: ticket(revision),
        query: format!("query-{revision}"),
        anchor: None,
        composing: false,
        suspended: false,
    })
}
#[test]
fn adjacent_observations_coalesce_without_regressing() {
    let hub = Arc::new(Hub::default());
    hub.post(changed(1));
    hub.post(changed(3));
    hub.post(changed(2));
    let events = hub.take_test_events();
    assert_eq!(events.len(), 1);
    assert!(
        matches!(&events[0], Event::Inline(InlineEvent::Changed { ticket: t, query, .. })
        if *t == ticket(3) && query == "query-3")
    );
}
#[test]
fn confirmation_and_cancellation_are_coalescing_barriers() {
    let hub = Arc::new(Hub::default());
    hub.post(changed(1));
    hub.post(Event::Inline(InlineEvent::Confirm(ticket(1))));
    hub.post(changed(2));
    hub.post(Event::Inline(InlineEvent::Cancelled {
        session: 7,
        reason: "test",
    }));
    hub.post(changed(3));
    let events = hub.take_test_events();
    assert_eq!(events.len(), 5);
    assert!(matches!(events[1], Event::Inline(InlineEvent::Confirm(t)) if t == ticket(1)));
    assert!(matches!(
        events[3],
        Event::Inline(InlineEvent::Cancelled { session: 7, .. })
    ));
}
#[test]
fn same_ticket_can_update_composition_without_dropping_other_sessions() {
    let hub = Arc::new(Hub::default());
    hub.post(changed(1));
    hub.post(Event::Inline(InlineEvent::Changed {
        ticket: ticket(1),
        query: "preedit".into(),
        anchor: None,
        composing: true,
        suspended: false,
    }));
    let mut other = ticket(2);
    other.session = 8;
    hub.post(Event::Inline(InlineEvent::Changed {
        ticket: other,
        query: String::new(),
        anchor: None,
        composing: false,
        suspended: false,
    }));
    let events = hub.take_test_events();
    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[0],
        Event::Inline(InlineEvent::Changed {
            composing: true,
            ..
        })
    ));
}
#[test]
fn bounded_batches_preserve_all_results_and_quit_in_order() {
    let hub = Arc::new(Hub::default());
    for i in 0..100 {
        hub.post(Event::Inspected(i, Err("test result".into())));
    }
    hub.post(Event::Command(Command::Quit));
    let mut received = 0;
    loop {
        let batch = hub.take_batch();
        if batch.is_empty() {
            break;
        }
        assert!(batch.len() <= 32);
        for event in batch {
            match event {
                Event::Inspected(serial, _) => {
                    assert_eq!(serial, received);
                    received += 1;
                }
                Event::Command(Command::Quit) => assert_eq!(received, 100),
                _ => panic!("unexpected event"),
            }
        }
    }
    assert_eq!(received, 100);
}
#[test]
fn closed_hub_rejects_late_deliveries() {
    let hub = Arc::new(Hub::default());
    hub.post(changed(1));
    hub.close();
    hub.post(changed(2));
    assert!(hub.take_test_events().is_empty());
}
