---
id: send_message
name: Send Message
triggers:
  - "send"
  - "tell"
  - "reply"
  - "say"
  - "message"
  - "text"
---

When the user wants to send a message to someone:

1. **send_message** → call send_message(contact="[person name]", message="[what to say]", app="[app name, default WhatsApp]")
2. **finish** → call finish(summary="Sent '[message]' to [contact] on [app]")

Extract from the user's request:
- contact = the person's name (e.g. "Mom", "Girlfriend", "John")
- message = what to send (e.g. "hi", "I'll be late", "今晚返屋企食飯")
- app = which app (default "WhatsApp" if not specified. Use "LINE", "Telegram", "Messages" if mentioned)
