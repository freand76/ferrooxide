// SystemVerilog packages: parameters, typedefs (enum, struct), functions, a package importing
// another, wildcard / explicit / header imports, pkg::name in expressions, types and casts
package base_pkg;
  localparam int W = 8;
  typedef logic [W-1:0] word_t;
  function automatic word_t inc(input word_t x);
    return x + 1;
  endfunction
endpackage

package bus_pkg;
  import base_pkg::*;
  localparam word_t MAGIC = 8'h5A;
  typedef enum logic [1:0] {IDLE, REQ, ACK} state_t;
  typedef struct packed {
    state_t st;
    word_t  data;
  } msg_t;
  function automatic msg_t mk(input state_t s, input word_t d);
    return '{s, d};
  endfunction
endpackage

import base_pkg::W;   // file scope: visible in every module below

module unit_user(output logic [W-1:0] o);
  assign o = {W{1'b1}};
endmodule

module hdr import base_pkg::*, bus_pkg::*; (input msg_t m, output word_t d);
  assign d = (m.st == ACK) ? m.data : MAGIC;
endmodule

module own;
  import bus_pkg::*;
  localparam int MAGIC = 3;   // the module's own name wins over the package's
  initial #5 $display("own MAGIC %0d", MAGIC);
endmodule

module tb;
  import bus_pkg::msg_t;
  msg_t m;
  base_pkg::word_t d, u;
  bus_pkg::state_t st;
  unit_user uu(.o(u));
  hdr h(.m(m), .d(d));
  own o();
  initial begin
    m = bus_pkg::mk(bus_pkg::REQ, 8'h11);
    #1 $display("m %h d %h u %h W %0d", m, d, u, W);
    m.st = bus_pkg::ACK;
    #1 $display("m %h d %h", m, d);
    st = bus_pkg::state_t'(2);
    case (st)
      bus_pkg::IDLE: $display("idle");
      bus_pkg::ACK:  $display("ack");
      default:       $display("other");
    endcase
    $display("inc %h magic %h bits %0d", base_pkg::inc(8'hFF), bus_pkg::MAGIC, $bits(bus_pkg::msg_t));
    #10 $finish;
  end
endmodule
