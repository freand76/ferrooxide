// Whole unpacked arrays (IEEE 1800-2017 7.6): continuous and procedural assignments pair
// the elements from the left; array ports connect arrays of the same shape.
module sum4(input wire [7:0] d[0:3], output logic [7:0] s, output logic [7:0] q[0:3]);
  always_comb begin
    s = d[0] + d[1] + d[2] + d[3];
    for (int i = 0; i < 4; i++) q[i] = d[3 - i];
  end
endmodule

module test;
  logic [7:0] a[0:3];
  logic [7:0] b[0:3];
  logic [7:0] c[4:1];
  logic [7:0] r[3:0];
  wire [7:0] w[0:3];
  wire [7:0] v[1:4];
  wire [7:0] s;
  wire [7:0] q[0:3];
  integer fails = 0;

  assign w = a;     // same shape
  assign v = a;     // another range: element by element
  sum4 u(.d(a), .s(s), .q(q));

  task check(input integer got, input integer want, input integer line);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d, want %0d", line, got, want);
      fails++;
    end
  endtask

  initial begin
    for (int i = 0; i < 4; i++) a[i] = 10 * (i + 1);
    #1;
    check(w[0], 10, `__LINE__); check(w[3], 40, `__LINE__);
    check(v[1], 10, `__LINE__); check(v[4], 40, `__LINE__);
    check(s, 100, `__LINE__); check(q[0], 40, `__LINE__); check(q[3], 10, `__LINE__);
    b = a;
    c = a;            // c[4] = a[0], ..., c[1] = a[3]
    r = a;            // r[3] = a[0], ..., r[0] = a[3]
    check(b[0], 10, `__LINE__); check(b[3], 40, `__LINE__);
    check(c[4], 10, `__LINE__); check(c[1], 40, `__LINE__);
    check(r[3], 10, `__LINE__); check(r[0], 40, `__LINE__);
    a[2] = 99;
    b <= a;
    check(b[2], 30, `__LINE__);
    #1;
    check(b[2], 99, `__LINE__); check(w[2], 99, `__LINE__); check(v[3], 99, `__LINE__);
    check(s, 169, `__LINE__);
    if (fails == 0) $display("PASSED");
  end
endmodule
