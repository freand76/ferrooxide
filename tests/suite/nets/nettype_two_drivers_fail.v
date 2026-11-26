// exit: 1
// a net of a nettype without a resolution function takes one driver (IEEE 1800-2017 6.6.7)
module top;
  nettype logic [7:0] byte_net;
  byte_net b;
  assign b = 8'h33;
  assign b = 8'h34;
endmodule
