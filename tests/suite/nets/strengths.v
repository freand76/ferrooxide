// Drive strengths (IEEE 1800 28.11-28.12): the stronger driver wins; equal strengths with
// different values give x; pullup/pulldown are weaker than gates and assignments.
module t;
  integer errors = 0;
  reg a, b, en;
  wire w1, w2, w3, w4, p;
  assign (strong1, strong0) w1 = a;
  assign (weak1, weak0) w1 = b;
  assign (pull1, pull0) w2 = a;
  assign (weak1, weak0) w2 = b;
  assign (weak1, weak0) w3 = a;
  assign (weak1, weak0) w3 = b;
  assign (strong1, highz0) w4 = a;          // drives only a 1
  assign w4 = b;
  pullup (p);
  bufif1 (p, a, en);                        // drives p when en is 1
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %b want %b", line, got[0], want[0]);
      errors = errors + 1;
    end
  endtask
  initial begin
    a = 0; b = 1; en = 0;
    #1;
    check(`__LINE__, w1, 0);                 // strong 0 over weak 1
    check(`__LINE__, w2, 0);                 // pull 0 over weak 1
    check(`__LINE__, w3, 1'bx);              // weak against weak
    check(`__LINE__, w4, 1);                 // a 0 from (strong1, highz0) is z: b alone
    check(`__LINE__, p, 1);                  // only the pullup
    en = 1;
    #1 check(`__LINE__, p, 0);               // the strong buffer over the pullup
    a = 1; b = 0;
    #1;
    check(`__LINE__, w1, 1);
    check(`__LINE__, w4, 1'bx);              // strong 1 against strong 0
    check(`__LINE__, p, 1);
    if (errors == 0) $display("PASSED");
  end
endmodule
