// Nets with several drivers (IEEE 1800 6.6, 28.12): a wire resolves equal strengths to x
// on a conflict, z yields to any value; wand/wor/triand/trior; tri0/tri1 pull z to 0/1;
// supply0/supply1 are constant.
module t;
  integer errors = 0;
  reg [3:0] a, b;
  wire [3:0] w;
  wand [3:0] wa;
  wor [3:0] wo;
  tri0 [3:0] t0;
  tri1 [3:0] t1;
  supply0 s0;
  supply1 s1;
  assign w = a, w = b;
  assign wa = a, wa = b;
  assign wo = a, wo = b;
  assign t0 = a, t0 = b;
  assign t1 = a, t1 = b;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %b want %b", line, got[3:0], want[3:0]);
      errors = errors + 1;
    end
  endtask
  initial begin
    a = 4'b01zz; b = 4'b0z1z;
    #1;
    check(`__LINE__, w, 4'b011z);              // z yields to a value
    check(`__LINE__, wa, 4'b011z);
    check(`__LINE__, wo, 4'b011z);
    check(`__LINE__, t0, 4'b0110);             // only z bits are pulled
    check(`__LINE__, t1, 4'b0111);
    a = 4'b0101; b = 4'b0011;
    #1;
    check(`__LINE__, w, 4'b0xx1);              // a conflict is x
    check(`__LINE__, wa, 4'b0001);
    check(`__LINE__, wo, 4'b0111);
    a = 4'bx0x1; b = 4'b0z1z;
    #1;
    check(`__LINE__, wa, 4'b00x1);            // 0 wins in a wand, x with 1 is x
    check(`__LINE__, wo, 4'bx011);            // 1 wins in a wor
    check(`__LINE__, {s0, s1}, 2'b01);
    if (errors == 0) $display("PASSED");
  end
endmodule
