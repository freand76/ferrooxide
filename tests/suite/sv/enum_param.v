// a parameter of an enum type is a value of that enum (IEEE 1800-2017 6.19.3): it may be
// assigned to a variable or member of the enum type without a cast
package p;
  typedef enum logic [2:0] { e_1, e_2, e_4, e_8 } size_e;
  typedef struct packed { size_e size; logic [3:0] addr; } req_s;
endpackage
module top;
  import p::*;
  localparam size_e bs = size_e'(3);
  localparam size_e bt = e_2;
  req_s r;
  size_e x;
  initial begin
    x = bs;
    r.size = bt;
    if (x.name() == "e_8" && r.size == e_2) begin
      r.size = bs;
      if (r.size == e_8) $display("PASSED");
      else $display("FAILED");
    end else $display("FAILED");
  end
endmodule
