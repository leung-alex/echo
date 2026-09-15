use super::*;
use echo_engine::InlineTicket;
use echo_windows::inline::InlineEvent;
#[test]
fn background_wait_keeps_bootstrap_events_and_wakes_without_slint() {
    let hub = Arc::new(Hub::default());
    for _ in 0..1000 {
        hub.post(Event::Invalidated);
    }
    hub.post(Event::Shell(ShellEvent::Activation(vec![
        "--background".into()
    ])));
    let waiter = hub.clone();
    let waiting = std::thread::spawn(move || waiter.wait_for_activation());
    hub.post(Event::Shell(ShellEvent::Open));
    assert!(waiting.join().unwrap());
    assert_eq!(hub.take_test_events().len(), 3);
}
#[test]
fn background_quit_never_requires_graphics() {
    let hub = Arc::new(Hub::default());
    hub.post(Event::Shell(ShellEvent::Activation(vec!["--quit".into()])));
    assert!(!hub.wait_for_activation());
}
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
#[test]
fn display_result_backpressure_keeps_control_and_shutdown_live() {
    use std::{sync::mpsc, time::Duration};
    let hub = Arc::new(Hub::default());
    let pixels = || {
        Event::Thumbnail(
            1,
            "synthetic".into(),
            Ok(super::PixelData {
                requested: Default::default(),
                width: 1536,
                height: 1536,
                rgba: vec![0; 9 * 1024 * 1024],
            }),
        )
    };
    hub.post(pixels());
    let producer = hub.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        entered_tx.send(()).unwrap();
        producer.post(pixels());
        done_tx.send(()).unwrap();
    });
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(done_rx.recv_timeout(Duration::from_millis(20)).is_err());
    hub.post(Event::Command(super::Command::Quit));
    assert!(!hub.wait_for_activation());
    hub.close();
    done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    thread.join().unwrap();
    assert!(hub.take_test_events().is_empty());
}
