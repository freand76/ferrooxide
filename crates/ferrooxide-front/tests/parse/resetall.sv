// `resetall between modules: the timescale, default net type and unconnected drive go
// back to their defaults
`timescale 1ns/1ps
`default_nettype none
`unconnected_drive pull1
module a(input wire x);
endmodule
`resetall
module b(input x);
endmodule
