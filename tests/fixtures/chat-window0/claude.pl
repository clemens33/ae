#!/usr/bin/perl
# Same stable composer shape as the launch suite's FAKE_AGENT. The harness
# stays idle while real ae delivery, pane stamps and window placement run.
use strict;
use warnings;
system("stty raw -echo 2>/dev/null");
binmode(STDIN, ':raw');
binmode(STDOUT, ':raw');
$| = 1;
print "\e[?2004h\e[H\e[2Jfake agent transcript\r\n";
print "\e[1m\xe2\x9d\xaf\e[0m\xc2\xa0\r\n";
print "\xe2\x94\x80" x 400, "\r\n  fake-model  ~/x\r\n";
while (1) { sleep 1; }
