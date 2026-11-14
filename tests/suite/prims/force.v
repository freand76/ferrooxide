// force/release and assign/deassign (IEEE 1800 10.6): a forced variable keeps the forced
// value against assignments until released (a variable then keeps it until the next
// assignment, a net returns to its drivers); force of a net or a bit select; the forced
// expression is followed while forced
module t;
  integer errors = 0;
  reg [3:0] r, src;
  wire [3:0] w;
  reg drv;
  assign w = {4{drv}};
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %b want %b", line, got[3:0], want[3:0]);
      errors = errors + 1;
    end
  endtask
  initial begin
    r = 1; drv = 0; src = 4'd9;
    #1 force r = 4'd5;
    #1 check(`__LINE__, r, 5);
    r = 2;                        // no effect while forced
    #1 check(`__LINE__, r, 5);
    release r;
    #1 check(`__LINE__, r, 5);    // a variable keeps the value...
    r = 3;
    #1 check(`__LINE__, r, 3);    // ...until assigned
    force w = src;                // follows src
    #1 check(`__LINE__, w, 9);
    src = 4'd6;
    #1 check(`__LINE__, w, 6);
    drv = 1;
    #1 check(`__LINE__, w, 6);
    release w;                    // the net takes its driver again
    #1 check(`__LINE__, w, 4'hf);
    force w[0] = 1'b0;            // one bit
    #1 check(`__LINE__, w, 4'he);
    release w[0];
    #1 check(`__LINE__, w, 4'hf);
    assign r = src + 1;           // procedural continuous assignment
    #1 check(`__LINE__, r, 7);
    src = 1;
    #1 check(`__LINE__, r, 2);
    deassign r;
    r = 0;
    #1 check(`__LINE__, r, 0);
    if (errors == 0) $display("PASSED");
  end
endmodule
