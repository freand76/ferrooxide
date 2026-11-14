// Enum methods: first/last/num/next/prev/name, values that are no member, %s of .name()
package p;
  typedef enum {RED, GREEN = 5, BLUE} color_t;
endpackage

module t;
  import p::*;
  typedef enum logic [2:0] {IDLE, RUN = 3, DONE} st_t;
  st_t s;
  color_t c;
  reg [8*6:1] r;
  integer i;
  initial begin
    r = "AB";
    $display("[%s] [%0s]", r, r);
    s = RUN;
    $display("[%s] [%0s] %0d %0d %0d", s.name(), s.name(), s.next(), s.prev(), s.next(2));
    $display("%0d %0d %0d", s.first(), s.last(), s.num());
    s = st_t'(5);
    $display("[%s] %0d %0d", s.name(), s.next(), s.prev());
    s = DONE;
    $display("[%s] %0d %0d %d", s.name(), s.next(), s.prev(), s.next());
    c = c.first();
    for (i = 0; i < 4; i = i + 1) begin
      $display("%0d: %s = %0d, prev %s", i, c.name(), c, c.prev() == RED ? "red" : "other");
      c = c.next();
    end
    c = c.last();
    $display("last [%8s] [%-6s] next(3) %0d", c.name(), c.name(), c.next(3));
  end
endmodule
